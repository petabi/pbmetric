use crate::git::{blame_stats, Repo};
use crate::github;
use crate::issue::{individual_stats, IndividualStats};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use std::cmp::{max, Ordering};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;
use std::process::exit;

const EXCLUDE_DEFAULT: [&str; 9] = [
    r"^\.git/",
    r"(^|/)Cargo\.lock$",
    r"\.dat$",
    r"\.log$",
    r"\.pcap$",
    r"\.png$",
    r"\.woff$",
    r"\.woff2$",
    r"^LICENSE$",
];

#[derive(Default, Deserialize)]
pub struct GithubConfig {
    token: String,
    repositories: Vec<String>,
    account: HashMap<String, String>,
}

#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
pub fn agenda<P: AsRef<Path>>(
    out: &mut dyn Write,
    github_conf: &GithubConfig,
    repo_root: P,
    repos: &BTreeMap<String, Repo>,
    email_map: &BTreeMap<String, String>,
    asof: &DateTime<Utc>,
    epoch: &Option<DateTime<Utc>>,
) -> Result<()> {
    out.write_all(b"<html><body>")?;

    let quarter_ago = *asof - Duration::try_days(90).expect("valid constant value");
    let since = match epoch {
        Some(epoch) => max(epoch, &quarter_ago),
        None => &quarter_ago,
    };

    let total_loc = repo_loc(repo_root.as_ref(), repos, since, asof);

    let github_api = github::Client::new(&github_conf.token);

    let pull_requests = github_api.open_pull_requests(&github_conf.repositories)?;
    write_pull_request_section(out, &pull_requests, &github_conf.account)?;

    let github_issues = github_api.assigned_stale_issues(&github_conf.repositories, asof)?;
    if !github_issues.is_empty() {
        write_issues_section(out, &github_issues, &github_conf.account)?;
    }

    let issue_metadata = github_api.issue_metadata_since(&github_conf.repositories, since)?;
    let week_ago = *asof - Duration::try_weeks(1).expect("valid constant value");
    let github_issue_stats =
        github_api.recent_issues_per_login(&github_conf.repositories, since, &week_ago)?;
    let created_count: usize = github_issue_stats.values().map(|v| v.3).sum();
    let authors = github_issue_stats
        .iter()
        .map(|(k, v)| (k.clone(), v.3))
        .collect::<BTreeMap<String, usize>>();
    let closed_count = github_issue_stats
        .values()
        .map(|v| v.4)
        .sum::<f32>()
        .round() as i64;
    let assignees = github_issue_stats
        .iter()
        .map(|(k, v)| (k.clone(), v.4))
        .collect::<BTreeMap<String, f32>>();

    out.write_all(b"<h2>Changes in the Past Week</h2>\n<ul>\n")?;
    out.write_all(format!("<li>Created: {created_count}\n<ul>\n").as_bytes())?;
    let mut authors = authors
        .iter()
        .map(|(username, count)| (*count, username))
        .collect::<Vec<(usize, &String)>>();
    authors.sort();
    for (count, username) in authors.iter().rev() {
        if *count == 0 {
            continue;
        }
        let username = github_conf.account.get(*username).unwrap_or(username);
        out.write_all(format!("<li>{username}: {count}\n").as_bytes())?;
    }
    out.write_all(b"</ul>\n")?;
    out.write_all(format!("<li>Completed: {closed_count}\n<ul>\n").as_bytes())?;
    let mut assignees = assignees
        .iter()
        .map(|(username, count)| (*count, username))
        .collect::<Vec<(f32, &String)>>();
    assignees.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Less));
    for (count, username) in assignees.iter().rev() {
        if *count == 0. {
            continue;
        }
        let username = github_conf.account.get(*username).unwrap_or(username);
        out.write_all(format!("<li>{username}: {count:.0}\n").as_bytes())?;
    }
    out.write_all(b"</ul>\n</ul>\n")?;

    let pull_requests =
        github_api.merged_pull_requests_per_login(&github_conf.repositories, since)?;
    out.write_all(b"\n<h2>Individual Statistics for the Past 90 Days</h2>\n<ul>")?;
    let mut stats = individual_stats(
        &issue_metadata,
        &pull_requests,
        &github_conf.account,
        since,
        asof,
    );
    for (email, loc) in &total_loc {
        let Some(username) = email_map.get(email) else {
            continue;
        };
        let entry = stats.entry(username.to_string()).or_default();
        entry.lines_contributed += loc;
    }
    for (username, stats) in stats {
        print_individual_stat(out, &username, &stats, since, asof)?;
    }
    out.write_all(b"</ul>\n")?;
    print_unknown_emails(out, &total_loc, email_map)?;
    out.write_all(b"</pre>\n")?;

    out.write_all(
        format!(
            r#"<footer>Generated by <a href="https://github.com/petabi/pbmetric">{}</a></footer>"#,
            concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"),)
        )
        .as_bytes(),
    )?;

    out.write_all(b"</body></html>")?;
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
        println!("Scanning {name}");
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
                eprintln!("cannot scan repositories: {e}");
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

fn write_pull_request_section(
    out: &mut dyn Write,
    pull_requests: &[github::PullRequest],
    account_map: &HashMap<String, String>,
) -> Result<()> {
    let pull_requests = pull_requests
        .iter()
        .filter(|pr| !pr.title.starts_with("[WIP]"))
        .collect::<Vec<_>>();
    if pull_requests.is_empty() {
        return Ok(());
    }
    out.write_all(b"<h2>Pull Requests Under Review</h2>\n<ul>")?;
    for pr in pull_requests {
        out.write_all(
            format!(
                r#"<li><a href="https://github.com/petabi/{repo}/pull/{num}">{repo}#{num}</a> {}"#,
                pr.title,
                repo = pr.repo,
                num = pr.number
            )
            .as_bytes(),
        )?;
        for reviewers in &pr.reviewers {
            let username = account_map.get(reviewers).unwrap_or(reviewers);
            out.write_all(format!(" @{username}").as_bytes())?;
        }
        for assignee in &pr.assignees {
            let username = account_map.get(assignee).unwrap_or(assignee);
            out.write_all(format!(" @{username}").as_bytes())?;
        }
    }
    out.write_all(b"</ul>\n")?;
    Ok(())
}

fn write_issues_section(
    out: &mut dyn Write,
    github_issues: &[github::Issue],
    account_map: &HashMap<String, String>,
) -> Result<()> {
    out.write_all(b"<h2>Assigned Issues with No Update in Past 24 Hours</h2>\n<ul>")?;
    for issue in github_issues {
        out.write_all(format!(r#"<li><a href="https://github.com/petabi/{repo}/issues/{num}">{repo}#{num}</a> {}"#, issue.title, repo = issue.repo, num = issue.number).as_bytes())?;
        for assignee in &issue.assignees {
            let username = account_map.get(assignee).unwrap_or(assignee);
            out.write_all(format!(" @{username}").as_bytes())?;
        }
        out.write_all(b"\n")?;
    }
    out.write_all(b"</ul>\n")?;
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
    out.write_all(format!("<li>{username}\n<ul>\n").as_bytes())?;
    out.write_all(
        format!(
            "<li>{:.3} issues completed per day\n",
            stats.issues_completed as f64 / days as f64
        )
        .as_bytes(),
    )?;
    out.write_all(
        format!(
            "<li>{:.3} issues (non-bug) opened per day\n",
            stats.issues_opened as f64 / days as f64
        )
        .as_bytes(),
    )?;
    out.write_all(
        format!(
            "<li>{:.3} bugs reported per day\n",
            stats.bugs_reported as f64 / days as f64
        )
        .as_bytes(),
    )?;
    out.write_all(
        format!(
            "<li>{:.3} pull/merge requests opened per day\n",
            stats.merged_merge_requests_opened as f64 / days as f64
        )
        .as_bytes(),
    )?;
    out.write_all(
        format!(
            "<li>{:5.2} comments per merge request\n",
            stats.merge_request_notes as f64 / stats.merged_merge_requests_opened as f64
        )
        .as_bytes(),
    )?;
    out.write_all(
        format!(
            "<li>{:5.2} lines of code contributed per day\n",
            stats.lines_contributed as f64 / days as f64
        )
        .as_bytes(),
    )?;
    out.write_all(b"</ul>\n")?;
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
        buf.write_all(format!("<li>{email}: {loc} lines contributed\n").as_bytes())?;
    }
    if buf.is_empty() {
        return Ok(false);
    }
    out.write_all(b"\n<h2>Other emails in commits</h2>\n<ul>\n")?;
    out.write_all(&buf)?;
    out.write_all(b"</ul>\n")?;
    Ok(true)
}
