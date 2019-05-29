use chrono::naive::NaiveDate;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use gitlab::{Gitlab, Issue, MergeRequest, Project, ProjectId};
use maplit::btreemap;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

pub fn stale_issues<'a>(
    api: &Gitlab,
    issues: &'a [Issue],
    asof: &DateTime<Utc>,
    projects: &mut HashMap<ProjectId, Project>,
) -> gitlab::Result<Vec<&'a Issue>> {
    let mut stale_issues = Vec::new();
    let params = HashMap::<&str, &str>::new();
    for issue in issues {
        if issue.updated_at > *asof - Duration::days(1) {
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

pub fn assignee_username(issue: &Issue) -> Option<&str> {
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

pub fn issues_opened(
    api: &Gitlab,
    project_ids: &[u64],
    asof: &DateTime<Utc>,
) -> gitlab::Result<Vec<Issue>> {
    let rfc3339_asof = asof.to_rfc3339_opts(SecondsFormat::Millis, true);
    let params = btreemap! { "state" => "opened", "created_before" => &rfc3339_asof };
    let mut issues = Vec::new();
    for id in project_ids {
        issues.extend(api.issues(ProjectId::new(*id), &params)?);
    }
    Ok(issues)
}

pub fn issues_updated_recently(
    api: &Gitlab,
    project_ids: &[u64],
    since: &DateTime<Utc>,
    asof: &DateTime<Utc>,
) -> gitlab::Result<Vec<Issue>> {
    let params = btreemap! {
        "updated_after" => since.to_string(),
        "created_before" => asof.to_rfc3339_opts(SecondsFormat::Millis, true),
    };
    let mut issues = Vec::new();
    for id in project_ids {
        issues.extend(api.issues(ProjectId::new(*id), &params)?);
    }
    Ok(issues)
}

pub fn merged_merge_requests_opened_recently(
    api: &Gitlab,
    project_ids: &[u64],
    since: &DateTime<Utc>,
    asof: &DateTime<Utc>,
) -> gitlab::Result<Vec<MergeRequest>> {
    let params = btreemap! {
        "created_after" => since.to_rfc3339_opts(SecondsFormat::Millis, true),
        "created_before" => asof.to_rfc3339_opts(SecondsFormat::Millis, true),
        "state" => "merged".to_string(),
    };
    let mut merge_requests = Vec::new();
    for id in project_ids {
        merge_requests.extend(api.merge_requests(ProjectId::new(*id), &params)?);
    }
    Ok(merge_requests)
}

pub fn issue_due_cmp(lhs: &Issue, rhs: &Issue) -> Ordering {
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

pub fn merge_requests_opened(
    api: &Gitlab,
    project_ids: &[u64],
    asof: &DateTime<Utc>,
) -> gitlab::Result<Vec<MergeRequest>> {
    let rfc3339_asof = asof.to_rfc3339_opts(SecondsFormat::Millis, true);
    let params =
        btreemap! { "state" => "opened", "wip" => "no", "created_before" => &rfc3339_asof };
    let mut merge_requests = Vec::new();
    for id in project_ids {
        merge_requests.extend(api.merge_requests(ProjectId::new(*id), &params)?);
    }
    Ok(merge_requests)
}

#[derive(Debug, Default)]
pub struct IndividualStats {
    pub bugs_reported: usize,
    pub issues_completed: usize,
    pub issues_opened: usize,
    pub merged_merge_requests_opened: usize,
    pub merge_request_notes: u64,
    pub lines_contributed: usize,
}

pub fn individual_stats(
    issues: &[Issue],
    merge_requests: &[MergeRequest],
    since: &DateTime<Utc>,
    asof: &DateTime<Utc>,
) -> BTreeMap<String, IndividualStats> {
    let mut stats = BTreeMap::new();
    for issue in issues {
        if *since < issue.created_at && issue.created_at < *asof {
            if issue.labels.contains(&"bug".to_string()) {
                let entry = stats
                    .entry(issue.author.username.clone())
                    .or_insert_with(IndividualStats::default);
                entry.bugs_reported += 1;
            } else {
                let entry = stats
                    .entry(issue.author.username.clone())
                    .or_insert_with(IndividualStats::default);
                entry.issues_opened += 1;
            }
        }
        if let Some(closed_at) = issue.closed_at {
            if *since < closed_at && closed_at < *asof {
                if let Some(username) = assignee_username(&issue) {
                    let entry = stats
                        .entry(username.to_string())
                        .or_insert_with(IndividualStats::default);
                    entry.issues_completed += 1;
                }
            }
        }
    }
    for mr in merge_requests {
        let entry = stats
            .entry(mr.author.username.clone())
            .or_insert_with(IndividualStats::default);
        entry.merged_merge_requests_opened += 1;
        entry.merge_request_notes += mr.user_notes_count;
    }
    stats
}
