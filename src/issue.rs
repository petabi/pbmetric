use gitlab::{Gitlab, Issue, ProjectId};
use maplit::btreemap;

pub fn agenda<S: ToString>(token: S, project_ids: &[u64]) -> gitlab::Result<()> {
    let issues = issues_opened(token, project_ids)?;
    print!("## Issues with Milestone\n\n");
    for issue in issues {
        let _milestone = match issue.milestone {
            Some(milestone) => milestone,
            None => continue,
        };
        println!("* {}", issue.title);
    }
    Ok(())
}

fn issues_opened<S: ToString>(token: S, project_ids: &[u64]) -> gitlab::Result<Vec<Issue>> {
    let api = Gitlab::new("gitlab.com", token)?;
    let params = btreemap! { "state" => "opened" };
    let mut issues = Vec::new();
    for id in project_ids {
        issues.extend(api.issues(ProjectId::new(*id), &params)?);
    }
    Ok(issues)
}
