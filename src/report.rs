use crate::git::{blame_stats, Repo};
use crate::github;
use crate::issue::{
    assignee_username, due_cmp, individual_stats, issues_opened, issues_updated_recently,
    merge_requests_opened, merged_merge_requests_opened_recently, stale_issues, IndividualStats,
};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use gitlab::{self, Gitlab};
use serde::Deserialize;
use std::cmp::max;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;
use std::process::exit;

const EXCLUDE_DEFAULT: [&str; 7] = [
    r#"^\.git/"#,
    r#"(^|/)Cargo\.lock$"#,
    r#"\.dat$"#,
    r#"\.log$"#,
    r#"\.pcap$"#,
    r#"\.png$"#,
    r#"^LICENSE$"#,
];

#[derive(Default, Deserialize)]
pub struct GithubConfig {
    token: String,
    repositories: Vec<String>,
    account: HashMap<String, String>,
}

#[derive(Default, Deserialize)]
pub struct GitlabConfig {
    token: String,
    projects: Vec<u64>,
    usernames: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
pub fn agenda<P: AsRef<Path>>(
    out: &mut dyn Write,
    github_conf: &GithubConfig,
    gitlab_conf: &GitlabConfig,
    repo_root: P,
    repos: &BTreeMap<String, Repo>,
    email_map: &BTreeMap<String, String>,
    asof: &DateTime<Utc>,
    epoch: &Option<DateTime<Utc>>,
) -> Result<()> {
    let quarter_ago = *asof - Duration::days(90);
    let since = match epoch {
        Some(epoch) => max(epoch, &quarter_ago),
        None => &quarter_ago,
    };

    let total_loc = repo_loc(repo_root.as_ref(), repos, &since, asof);

    let github_api = github::Client::new(&github_conf.token);
    let gitlab_api = Gitlab::new("gitlab.com", &gitlab_conf.token)?;

    let pull_requests = github_api.open_pull_requests(&github_conf.repositories)?;
    let merge_requests = merge_requests_opened(&gitlab_api, &gitlab_conf.projects, asof)?;
    let mut projects = if merge_requests.is_empty() {
        write_merge_request_section(out, &pull_requests, &merge_requests, &gitlab_api)?;
        HashMap::new()
    } else {
        write_merge_request_section(out, &pull_requests, &merge_requests, &gitlab_api)?
    };

    let mut issues = issues_opened(&gitlab_api, &gitlab_conf.projects, asof)?;
    issues.sort_by(due_cmp);
    let stale_issues = stale_issues(&gitlab_api, &issues, asof, &mut projects)?;
    if !stale_issues.is_empty() {
        write_issues_section(out, &projects, &stale_issues)?;
    }

    let week_ago = *asof - Duration::weeks(1);
    let issues = issues_updated_recently(&gitlab_api, &gitlab_conf.projects, &since, asof)?;
    let mut created_count = 0_usize;
    let mut authors = BTreeMap::new();
    let mut closed_count = 0_usize;
    let mut assignees = BTreeMap::new();
    for issue in &issues {
        if issue.updated_at < week_ago {
            continue;
        }
        if week_ago < issue.created_at && issue.created_at < *asof {
            let entry = authors
                .entry(issue.author.username.clone())
                .or_insert(0_usize);
            *entry += 1;
            created_count += 1;
        }
        if let Some(closed_at) = issue.closed_at {
            if week_ago < closed_at && closed_at < *asof {
                if let Some(username) = assignee_username(&issue) {
                    let entry = assignees.entry(username.to_string()).or_insert(0_usize);
                    *entry += 1;
                    closed_count += 1;
                }
            }
        }
    }

    out.write_all(b"\n## Changes in the Past Week\n\n")?;
    out.write_all(format!("* Created: {}\n", created_count).as_bytes())?;
    let mut authors = authors
        .iter()
        .map(|(username, count)| (*count, username))
        .collect::<Vec<(usize, &String)>>();
    authors.sort();
    for (count, username) in authors.iter().rev() {
        out.write_all(format!("  - {}: {}\n", username, count).as_bytes())?;
    }
    out.write_all(format!("\n* Completed: {}\n", closed_count).as_bytes())?;
    let mut assignees = assignees
        .iter()
        .map(|(username, count)| (*count, username))
        .collect::<Vec<(usize, &String)>>();
    assignees.sort();
    for (count, username) in assignees.iter().rev() {
        out.write_all(format!("  - {}: {}\n", username, count).as_bytes())?;
    }

    let pull_requests =
        github_api.merged_pull_requests_per_login(&github_conf.repositories, &since)?;
    let merge_requests =
        merged_merge_requests_opened_recently(&gitlab_api, &gitlab_conf.projects, &since, &asof)?;
    out.write_all(b"\n## Individual Statistics for the Past 90 Days\n\n")?;
    let mut stats = individual_stats(
        &issues,
        &pull_requests,
        &github_conf.account,
        &merge_requests,
        &since,
        &asof,
    );
    for (email, loc) in &total_loc {
        let username = match email_map.get(email) {
            Some(username) => username,
            None => continue,
        };
        let entry = stats
            .entry(username.to_string())
            .or_insert_with(IndividualStats::default);
        entry.lines_contributed += loc;
    }
    for (username, stats) in stats {
        if !gitlab_conf.usernames.contains(&username) {
            continue;
        }
        print_individual_stat(out, &username, &stats, &since, asof)?;
    }
    print_unknown_emails(out, &total_loc, email_map)?;
    out.write_all(
        format!(
            "\n\nGenerated by {}",
            concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"),)
        )
        .as_bytes(),
    )?;

    Ok(())
}

fn repo_loc(
    root: &Path,
    repos: &BTreeMap<String, Repo>,
    start_date: &DateTime<Utc>,
    end_date: &DateTime<Utc>,
) -> HashMap<String, usize> {
    let mut total_loc = HashMap::new();
    let mut path = root.to_path_buf();
    for (name, repo) in repos {
        path.push(name);
        println!("Scanning {}", name);
        let mut exclude = EXCLUDE_DEFAULT
            .iter()
            .map(|e| (*e).to_string())
            .collect::<Vec<String>>();
        if let Some(repo_exclude) = &repo.exclude {
            exclude.extend(repo_exclude.iter().cloned());
        }
        let blame_stats = match blame_stats(&path, start_date, end_date, exclude) {
            Ok(stats) => stats,
            Err(e) => {
                eprintln!("cannot scan repositories: {}", e);
                exit(1);
            }
        };
        for (email, loc) in blame_stats {
            let entry = total_loc.entry(email).or_insert(0);
            *entry += loc;
        }
        path.pop();
    }
    total_loc
}

fn write_merge_request_section(
    out: &mut dyn Write,
    pull_requests: &[github::PullRequest],
    merge_requests: &[gitlab::MergeRequest],
    api: &Gitlab,
) -> Result<HashMap<gitlab::ProjectId, gitlab::Project>> {
    let mut projects = HashMap::new();
    if pull_requests.is_empty() && merge_requests.is_empty() {
        return Ok(projects);
    }
    out.write_all(b"\n## Pull/Merge Requests Under Review\n\n")?;
    for pr in pull_requests {
        out.write_all(format!("* {}#{} {}", pr.repo, pr.number, pr.title).as_bytes())?;
        for assignee in &pr.assignees {
            out.write_all(format!(" @{}", assignee).as_bytes())?;
        }
        out.write_all(b"\n")?;
    }
    for mr in merge_requests {
        let project = if let Some(project) = projects.get(&mr.project_id) {
            project
        } else {
            let params = HashMap::<&str, &str>::new();
            projects.insert(mr.project_id, api.project(mr.project_id, &params)?);
            &projects[&mr.project_id]
        };
        out.write_all(format!("* {}!{} {}", project.path, mr.iid, mr.title).as_bytes())?;
        if let Some(assignee) = &mr.assignee {
            out.write_all(format!(" @{}", assignee.username).as_bytes())?;
        }
        out.write_all(b"\n")?;
    }
    Ok(projects)
}

fn write_issues_section(
    out: &mut dyn Write,
    projects: &HashMap<gitlab::ProjectId, gitlab::Project>,
    stale_issues: &[&gitlab::Issue],
) -> Result<()> {
    out.write_all(b"\n## Assigned Issues with No Update in Past 24 Hours\n\n")?;
    for issue in stale_issues {
        let project = &projects[&issue.project_id];
        let assignee = assignee_username(issue).unwrap();
        out.write_all(
            format!(
                "* {}#{} {} @{}\n",
                project.path, issue.iid, issue.title, assignee
            )
            .as_bytes(),
        )?;
    }
    Ok(())
}

#[allow(clippy::cast_precision_loss)]
fn print_individual_stat(
    out: &mut dyn Write,
    username: &str,
    stats: &IndividualStats,
    since: &DateTime<Utc>,
    asof: &DateTime<Utc>,
) -> Result<()> {
    let days = (*asof - *since).num_days();
    out.write_all(format!("* {}\n", username).as_bytes())?;
    out.write_all(
        format!(
            "  - {:.3} issues completed per day\n",
            stats.issues_completed as f64 / days as f64
        )
        .as_bytes(),
    )?;
    out.write_all(
        format!(
            "  - {:.3} issues (non-bug) opened per day\n",
            stats.issues_opened as f64 / days as f64
        )
        .as_bytes(),
    )?;
    out.write_all(
        format!(
            "  - {:.3} bugs reported per day\n",
            stats.bugs_reported as f64 / days as f64
        )
        .as_bytes(),
    )?;
    out.write_all(
        format!(
            "  - {:.3} pull/merge requests opened per day\n",
            stats.merged_merge_requests_opened as f64 / days as f64
        )
        .as_bytes(),
    )?;
    out.write_all(
        format!(
            "  - {:5.2} comments per merge request\n",
            stats.merge_request_notes as f64 / stats.merged_merge_requests_opened as f64
        )
        .as_bytes(),
    )?;
    out.write_all(
        format!(
            "  - {:5.2} lines of code contributed per day\n",
            stats.lines_contributed as f64 / days as f64
        )
        .as_bytes(),
    )?;
    Ok(())
}

fn print_unknown_emails(
    out: &mut dyn Write,
    total_loc: &HashMap<String, usize>,
    email_map: &BTreeMap<String, String>,
) -> Result<bool> {
    let mut buf = Vec::<u8>::new();
    for (email, loc) in total_loc {
        if email_map.contains_key(email) {
            continue;
        }
        buf.write_all(format!("* {}: {} lines contributed\n", email, loc).as_bytes())?;
    }
    if buf.is_empty() {
        return Ok(false);
    }
    out.write_all(b"\n## Other emails in commits\n\n")?;
    out.write_all(&buf)?;
    Ok(true)
}
