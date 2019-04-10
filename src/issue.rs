use chrono::naive::NaiveDate;
use gitlab::{Gitlab, Issue, Milestone, Project, ProjectId};
use maplit::btreemap;
use std::cmp::Ordering;
use std::collections::HashMap;

pub fn agenda<S: ToString>(token: S, project_ids: &[u64]) -> gitlab::Result<()> {
    let api = Gitlab::new("gitlab.com", token)?;
    let mut issues = issues_opened(&api, project_ids)?;
    issues.sort_by(issue_due_cmp);
    let mut projects = HashMap::new();
    if let Err(e) = next_issues(&api, &issues, &mut projects) {
        return Err(e);
    }
    println!();
    abandoned_issues(&api, &issues, &mut projects)
}

fn next_issues(
    api: &Gitlab,
    issues: &[Issue],
    projects: &mut HashMap<ProjectId, Project>,
) -> gitlab::Result<()> {
    print!("## Next Milestone\n\n");
    let params = HashMap::<&str, &str>::new();
    let mut cur_milestone: Option<&Milestone> = None;
    for issue in issues {
        match &issue.milestone {
            Some(milestone) => {
                if let Some(cur) = &cur_milestone {
                    if cur.id != milestone.id {
                        break;
                    }
                } else {
                    cur_milestone = Some(milestone);
                }
            }
            None => continue,
        };
        let project = if let Some(project) = projects.get(&issue.project_id) {
            project
        } else {
            projects.insert(issue.project_id, api.project(issue.project_id, &params)?);
            &projects[&issue.project_id]
        };
        print!("* {}#{} {}", project.path, issue.iid, issue.title);
        if let Some(assignees) = &issue.assignees {
            for assignee in assignees {
                print!(" @{}", assignee.username);
            }
        }
        println!();
    }
    Ok(())
}

fn abandoned_issues(
    api: &Gitlab,
    issues: &[Issue],
    projects: &mut HashMap<ProjectId, Project>,
) -> gitlab::Result<()> {
    print!("## Assigned Issues with No Recent Activity\n\n");
    let params = HashMap::<&str, &str>::new();
    for issue in issues {
        if issue.updated_at > chrono::Utc::now() - chrono::Duration::weeks(1) {
            continue;
        }
        let assignee = match &issue.assignees {
            Some(assignees) => {
                if let Some(assignee) = assignees.first() {
                    &assignee.username
                } else {
                    continue;
                }
            }
            None => continue,
        };
        let project = if let Some(project) = projects.get(&issue.project_id) {
            project
        } else {
            projects.insert(issue.project_id, api.project(issue.project_id, &params)?);
            &projects[&issue.project_id]
        };
        println!(
            "* {}#{} {} @{}",
            project.path, issue.iid, issue.title, assignee
        );
    }
    Ok(())
}

fn issues_opened(api: &Gitlab, project_ids: &[u64]) -> gitlab::Result<Vec<Issue>> {
    let params = btreemap! { "state" => "opened" };
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
