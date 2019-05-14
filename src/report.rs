use chrono::{DateTime, Duration, Utc};
use gitlab::Gitlab;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::cmp::max;
use std::process::exit;

use crate::git::{blame_stats, Repo};
use crate::issue::{
    assignee_username, individual_stats, issue_due_cmp, issues_opened, issues_updated_recently,
    merge_requests_opened, merged_merge_requests_opened_recently, stale_issues, IndividualStats,
};

pub fn agenda<S: ToString, P: AsRef<Path>>(
    token: S,
    project_ids: &[u64],
    usernames: &[String],
    repo_root: P,
    repos: &BTreeMap<String, Repo>,
    email_map: &BTreeMap<String, String>,
    epoch: &Option<DateTime<Utc>>,
) -> gitlab::Result<()> {
    let quarter_ago = Utc::now() - Duration::days(90);
    let since = match epoch {
        Some(epoch) => max(epoch, &quarter_ago),
        None => &quarter_ago,
    };

    let mut total_loc = HashMap::new();
    let mut path = repo_root.as_ref().to_path_buf();
    let exclude_default = vec![
        r#"^\.git/"#,
        r#"(^|/)Cargo\.lock$"#,
        r#"\.dat$"#,
        r#"\.log$"#,
        r#"\.pcap$"#,
    ];
    for (name, repo) in repos {
        path.push(name);
        println!("Scanning {}", name);
        let mut exclude = exclude_default
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<String>>();
        if let Some(repo_exclude) = &repo.exclude {
            exclude.extend(repo_exclude.iter().cloned());
        }
        let blame_stats = match blame_stats(&path, &since, exclude) {
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

    let api = Gitlab::new("gitlab.com", token)?;
    let mut projects = HashMap::new();

    let merge_requests = merge_requests_opened(&api, project_ids)?;
    if !merge_requests.is_empty() {
        print!("\n## Merge Requests Under Review\n\n");
        for mr in merge_requests {
            let project = if let Some(project) = projects.get(&mr.project_id) {
                project
            } else {
                let params = HashMap::<&str, &str>::new();
                projects.insert(mr.project_id, api.project(mr.project_id, &params)?);
                &projects[&mr.project_id]
            };
            print!("* {}!{} {}", project.path, mr.iid, mr.title);
            if let Some(assignee) = &mr.assignee {
                print!(" @{}", assignee.username);
            }
            println!();
        }
    }

    let mut issues = issues_opened(&api, project_ids)?;
    issues.sort_by(issue_due_cmp);
    let stale_issues = stale_issues(&api, &issues, &mut projects)?;
    if !stale_issues.is_empty() {
        print!("\n## Assigned Issues with No Update in Past 24 Hours\n\n");
        for issue in stale_issues {
            let project = &projects[&issue.project_id];
            let assignee = assignee_username(issue).unwrap();
            println!(
                "* {}#{} {} @{}",
                project.path, issue.iid, issue.title, assignee
            );
        }
    }

    let week_ago = Utc::now() - Duration::weeks(1);
    let issues = issues_updated_recently(&api, &since, project_ids)?;
    let mut created_count = 0usize;
    let mut authors = BTreeMap::new();
    let mut closed_count = 0usize;
    let mut assignees = BTreeMap::new();
    for issue in &issues {
        if issue.updated_at < week_ago {
            continue;
        }
        if week_ago < issue.created_at {
            let entry = authors
                .entry(issue.author.username.clone())
                .or_insert(0usize);
            *entry += 1;
            created_count += 1;
        }
        if let Some(closed_at) = issue.closed_at {
            if week_ago < closed_at {
                if let Some(username) = assignee_username(&issue) {
                    let entry = assignees.entry(username.to_string()).or_insert(0usize);
                    *entry += 1;
                    closed_count += 1;
                }
            }
        }
    }

    print!("\n## Changes in the Past Week\n\n");
    println!("* Created: {}", created_count);
    let mut authors = authors
        .iter()
        .map(|(username, count)| (*count, username))
        .collect::<Vec<(usize, &String)>>();
    authors.sort();
    for (count, username) in authors.iter().rev() {
        println!("  - {}: {}", username, count);
    }
    println!();
    println!("* Completed: {}", closed_count);
    let mut assignees = assignees
        .iter()
        .map(|(username, count)| (*count, username))
        .collect::<Vec<(usize, &String)>>();
    assignees.sort();
    for (count, username) in assignees.iter().rev() {
        println!("  - {}: {}", username, count);
    }

    let merge_requests = merged_merge_requests_opened_recently(&api, &since, project_ids)?;
    print!("\n## Individual Statistics for the Past 90 Days\n\n");
    let mut stats = individual_stats(&issues, &merge_requests, &since);
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
        if !usernames.contains(&username) {
            continue;
        }
        print_individual_stat(&username, &stats, &since);
    }
    print_unknown_emails(&total_loc, email_map);

    Ok(())
}

fn print_individual_stat(username: &str, stats: &IndividualStats, since: &DateTime<Utc>) {
    let days = (Utc::now() - *since).num_days();
    println!("* {}", username);
    println!(
        "  - {:.3} issues completed per day",
        stats.issues_completed as f64 / days as f64
    );
    println!(
        "  - {:.3} issues (non-bug) opened per day",
        stats.issues_opened as f64 / days as f64
    );
    println!(
        "  - {:.3} bugs reported per day",
        stats.bugs_reported as f64 / days as f64
    );
    println!(
        "  - {:.3} merge requests opened per day",
        stats.merged_merge_requests_opened as f64 / days as f64
    );
    println!(
        "  - {:5.2} comments per merge request",
        stats.merge_request_notes as f64 / stats.merged_merge_requests_opened as f64
    );
    println!(
        "  - {:5.2} lines of code contributed per day",
        stats.lines_contributed as f64 / days as f64
    );
}

fn print_unknown_emails(total_loc: &HashMap<String, usize>, email_map: &BTreeMap<String, String>) {
    print!("\n## Other emails in commits\n\n");
    for (email, loc) in total_loc {
        if email_map.contains_key(email) {
            continue;
        }
        println!("* {}: {} lines contributed", email, loc);
    }
}
