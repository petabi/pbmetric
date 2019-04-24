use chrono::naive::NaiveDate;
use chrono::{DateTime, Duration, Utc};
use gitlab::{Gitlab, Issue, MergeRequest, Project, ProjectId};
use maplit::btreemap;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

pub fn agenda<S: ToString>(token: S, project_ids: &[u64]) -> gitlab::Result<()> {
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

    let since = Utc::now() - Duration::weeks(1);
    let issues = issues_updated_recently(&api, &since, project_ids)?;
    let mut created_count = 0usize;
    let mut authors = BTreeMap::new();
    let mut closed_count = 0usize;
    let mut assignees = BTreeMap::new();
    for issue in issues {
        if since < issue.created_at {
            let entry = authors
                .entry(issue.author.username.clone())
                .or_insert(0usize);
            *entry += 1;
            created_count += 1;
        }
        if let Some(closed_at) = issue.closed_at {
            if since < closed_at {
                if let Some(username) = assignee_username(&issue) {
                    let entry = assignees.entry(username.to_string()).or_insert(0usize);
                    *entry += 1;
                }
                closed_count += 1;
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
    Ok(())
}

fn stale_issues<'a>(
    api: &Gitlab,
    issues: &'a [Issue],
    projects: &mut HashMap<ProjectId, Project>,
) -> gitlab::Result<Vec<&'a Issue>> {
    let mut stale_issues = Vec::new();
    let params = HashMap::<&str, &str>::new();
    for issue in issues {
        if issue.updated_at > Utc::now() - Duration::days(1) {
            continue;
        }
        if issue.labels.contains(&"blocked".to_string()) {
            continue;
        }
        if assignee_username(issue).is_none() {
            continue;
        }
        projects
            .entry(issue.project_id)
            .or_insert(api.project(issue.project_id, &params)?);
        stale_issues.push(issue);
    }
    Ok(stale_issues)
}

fn assignee_username(issue: &Issue) -> Option<&str> {
    if let Some(assignees) = &issue.assignees {
        // GitLab CE allows one assignee only.DateTime
        if let Some(assignee) = assignees.first() {
            Some(&assignee.username)
        } else {
            None
        }
    } else {
        None
    }
}

fn issues_opened(api: &Gitlab, project_ids: &[u64]) -> gitlab::Result<Vec<Issue>> {
    let params = btreemap! { "state" => "opened" };
    let mut issues = Vec::new();
    for id in project_ids {
        issues.extend(api.issues(ProjectId::new(*id), &params)?);
    }
    Ok(issues)
}

fn issues_updated_recently(
    api: &Gitlab,
    since: &DateTime<Utc>,
    project_ids: &[u64],
) -> gitlab::Result<Vec<Issue>> {
    let params = btreemap! {"updated_after" => since.to_string() };
    let mut issues = Vec::new();
    for id in project_ids {
        issues.extend(api.issues(ProjectId::new(*id), &params)?);
    }
    Ok(issues)
}

fn issue_due_cmp(lhs: &Issue, rhs: &Issue) -> Ordering {
    if let Some(lhs_date) = issue_due_date(lhs) {
        if let Some(rhs_date) = issue_due_date(rhs) {
            let order = lhs_date.cmp(&rhs_date);
            if order == Ordering::Equal {
                lhs.updated_at.cmp(&rhs.updated_at)
            } else {
                order
            }
        } else {
            Ordering::Less
        }
    } else if issue_due_date(rhs).is_some() {
        Ordering::Greater
    } else {
        lhs.updated_at.cmp(&rhs.updated_at)
    }
}

fn issue_due_date(issue: &Issue) -> Option<NaiveDate> {
    if let Some(date) = issue.due_date {
        Some(date)
    } else if let Some(milestone) = &issue.milestone {
        if let Some(date) = milestone.due_date {
            Some(date)
        } else {
            None
        }
    } else {
        None
    }
}

fn merge_requests_opened(api: &Gitlab, project_ids: &[u64]) -> gitlab::Result<Vec<MergeRequest>> {
    let params = btreemap! { "state" => "opened", "wip" => "no" };
    let mut merge_requests = Vec::new();
    for id in project_ids {
        merge_requests.extend(api.merge_requests(ProjectId::new(*id), &params)?);
    }
    Ok(merge_requests)
}
