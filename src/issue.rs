use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};

use crate::github::IssueMetadata;

#[derive(Debug, Default)]
pub struct IndividualStats {
    pub bugs_reported: usize,
    pub issues_completed: usize,
    pub issues_opened: usize,
    pub merged_merge_requests_opened: usize,
    pub merge_request_notes: u64,
    pub lines_contributed: usize,
}

#[allow(clippy::cast_sign_loss)]
pub fn individual_stats(
    issues: &[IssueMetadata],
    pull_requests: &HashMap<String, (usize, i64)>,
    account_map: &HashMap<String, String>,
    since: &DateTime<Utc>,
    asof: &DateTime<Utc>,
) -> BTreeMap<String, IndividualStats> {
    let mut stats = BTreeMap::new();
    for issue in issues {
        if *since < issue.created_at && issue.created_at < *asof {
            let Some(author) = account_map.get(&issue.author) else {
                continue;
            };

            let entry = stats
                .entry(author.clone())
                .or_insert_with(IndividualStats::default);
            if issue.labels.contains(&"bug".to_string()) {
                entry.bugs_reported += 1;
            } else {
                entry.issues_opened += 1;
            }
        }
        if let Some(closed_at) = issue.closed_at {
            if *since < closed_at && closed_at < *asof {
                for assignee in &issue.assignees {
                    let Some(id) = account_map.get(assignee) else {
                        continue;
                    };
                    let entry = stats
                        .entry(id.clone())
                        .or_insert_with(IndividualStats::default);
                    entry.issues_completed += 1;
                }
            }
        }
    }
    for (login, count) in pull_requests {
        let Some(author) = account_map.get(login) else {
            continue;
        };
        let entry = stats
            .entry(author.clone())
            .or_insert_with(IndividualStats::default);
        entry.merged_merge_requests_opened += count.0;
        entry.merge_request_notes += count.1 as u64;
    }
    stats
}
