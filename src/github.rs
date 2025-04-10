use std::collections::HashMap;

use anyhow::Result;
use graphql_client::GraphQLQuery;

type DateTime = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github.graphql",
    query_path = "src/assigned_issues.graphql",
    response_derives = "Debug"
)]
struct AssignedIssues;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github.graphql",
    query_path = "src/recent_issues.graphql",
    response_derives = "Debug"
)]
struct RecentIssues;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github.graphql",
    query_path = "src/open_pull_requests.graphql",
    response_derives = "Debug"
)]
struct OpenPullRequests;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/github.graphql",
    query_path = "src/merged_pull_requests.graphql"
)]
struct MergedPullRequests;

static USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

pub struct Client {
    token: HashMap<String, String>,
    inner: reqwest::blocking::Client,
}

impl Client {
    pub fn new(token: HashMap<String, String>) -> Self {
        Self {
            token,
            inner: reqwest::blocking::ClientBuilder::new()
                .user_agent(USER_AGENT)
                .build()
                .unwrap(),
        }
    }

    pub fn assigned_stale_issues(
        &self,
        repos: &[String],
        asof: &chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Issue>> {
        let mut issues = Vec::new();
        for repo in repos {
            let (owner, name) = repo
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("Invalid repository format: {}", repo))?;
            let query = AssignedIssues::build_query(assigned_issues::Variables {
                owner: owner.to_string(),
                name: name.to_string(),
            });
            let res = self
                .inner
                .post("https://api.github.com/graphql")
                .bearer_auth(&self.token[owner])
                .json(&query)
                .send()?;

            let body: graphql_client::Response<assigned_issues::ResponseData> = res.json()?;
            if let Some(data) = body.data {
                if let Some(repository) = data.repository {
                    if let Some(nodes) = repository.issues.nodes {
                        for node in nodes {
                            let Some(node) = node else {
                                continue;
                            };
                            let updated_at =
                                chrono::DateTime::parse_from_rfc3339(&node.updated_at)?;
                            if updated_at
                                > *asof
                                    - chrono::Duration::try_days(1).expect("valid constant value")
                            {
                                continue;
                            }
                            issues.push(Issue {
                                title: node.title,
                                number: node.number,
                                repo: repo.to_string(),
                                assignees: node.assignees.nodes.map_or_else(Vec::new, |nodes| {
                                    nodes
                                        .into_iter()
                                        .filter_map(|v| v.map(|node| node.login))
                                        .collect()
                                }),
                            });
                        }
                    }
                }
            }
        }
        Ok(issues)
    }

    pub fn issue_metadata_since(
        &self,
        repos: &[String],
        since: &chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<IssueMetadata>> {
        let mut issues = Vec::new();
        let rfc3339_since = since.to_rfc3339();
        for repo in repos {
            let (owner, name) = repo
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("Invalid repository format: {}", repo))?;
            let query = RecentIssues::build_query(recent_issues::Variables {
                owner: owner.to_string(),
                name: name.to_string(),
                since: rfc3339_since.clone(),
            });
            let res = self
                .inner
                .post("https://api.github.com/graphql")
                .bearer_auth(&self.token[owner])
                .json(&query)
                .send()?;

            let body: graphql_client::Response<recent_issues::ResponseData> = res.json()?;
            if let Some(data) = body.data {
                if let Some(repository) = data.repository {
                    if let Some(nodes) = repository.issues.nodes {
                        for node in nodes.into_iter().flatten() {
                            let author = node
                                .author
                                .map_or_else(|| "unknown".to_string(), |v| v.login);
                            let created_at =
                                chrono::DateTime::parse_from_rfc3339(&node.created_at)?;
                            let labels = node.labels.map_or_else(Vec::new, |labels| {
                                labels.nodes.map_or_else(Vec::new, |nodes| {
                                    nodes
                                        .into_iter()
                                        .filter_map(|v| v.map(|v| v.name))
                                        .collect()
                                })
                            });
                            let closed_at = if let Some(closed_at) = node.closed_at {
                                Some(chrono::DateTime::parse_from_rfc3339(&closed_at)?)
                            } else {
                                None
                            };
                            let assignees = node.assignees.nodes.map_or_else(Vec::new, |nodes| {
                                nodes
                                    .into_iter()
                                    .filter_map(|v| v.map(|v| v.login))
                                    .collect()
                            });
                            issues.push(IssueMetadata {
                                author,
                                labels,
                                assignees,
                                created_at,
                                closed_at,
                            });
                        }
                    }
                }
            }
        }
        Ok(issues)
    }

    #[allow(clippy::type_complexity)]
    pub fn recent_issues_per_login(
        &self,
        repos: &[String],
        since: &chrono::DateTime<chrono::Utc>,
        recent_since: &chrono::DateTime<chrono::Utc>,
    ) -> Result<HashMap<String, (usize, usize, f32, usize, f32)>> {
        let mut counter = HashMap::new();
        let rfc3339_since = since.to_rfc3339();
        for repo in repos {
            let (owner, name) = repo
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("Invalid repository format: {}", repo))?;
            let query = RecentIssues::build_query(recent_issues::Variables {
                owner: owner.to_string(),
                name: name.to_string(),
                since: rfc3339_since.clone(),
            });
            let res = self
                .inner
                .post("https://api.github.com/graphql")
                .bearer_auth(&self.token[owner])
                .json(&query)
                .send()?;

            let body: graphql_client::Response<recent_issues::ResponseData> = res.json()?;
            if let Some(data) = body.data {
                if let Some(repository) = data.repository {
                    if let Some(nodes) = repository.issues.nodes {
                        for node in nodes.into_iter().flatten() {
                            let created_at =
                                chrono::DateTime::parse_from_rfc3339(&node.created_at)?;
                            if *since <= created_at {
                                let author = node
                                    .author
                                    .map_or_else(|| "unknown".to_string(), |v| v.login);
                                let stat = counter.entry(author).or_insert((0, 0, 0.0, 0, 0.0));
                                if let Some(labels) = node.labels {
                                    if let Some(nodes) = labels.nodes {
                                        let is_bug = nodes
                                            .into_iter()
                                            .any(|v| v.is_some_and(|v| v.name == "bug"));
                                        if is_bug {
                                            stat.1 += 1;
                                        }
                                    } else {
                                        stat.0 += 1;
                                    }
                                } else {
                                    stat.0 += 1;
                                }

                                if *recent_since < created_at {
                                    stat.3 += 1;
                                }
                            }
                            if let Some(closed_at) = node.closed_at {
                                let closed_at = chrono::DateTime::parse_from_rfc3339(&closed_at)?;
                                if let Some(nodes) = node.assignees.nodes {
                                    let mut total_assignees = 0.0;
                                    for node in &nodes {
                                        if node.is_some() {
                                            total_assignees += 1.0;
                                        }
                                    }
                                    for node in nodes {
                                        let Some(node) = node else {
                                            continue;
                                        };
                                        let stat = counter
                                            .entry(node.login)
                                            .or_insert((0, 0, 0.0, 0, 0.0));
                                        stat.2 += 1.0 / total_assignees;

                                        if *recent_since < closed_at {
                                            stat.4 += 1.0 / total_assignees;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(counter)
    }

    pub fn open_pull_requests(&self, repos: &[String]) -> Result<Vec<PullRequest>> {
        let mut prs = Vec::new();
        for repo in repos {
            let (owner, name) = repo
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("Invalid repository format: {}", repo))?;
            let query = OpenPullRequests::build_query(open_pull_requests::Variables {
                owner: owner.to_string(),
                name: name.to_string(),
            });
            let res = self
                .inner
                .post("https://api.github.com/graphql")
                .bearer_auth(&self.token[owner])
                .json(&query)
                .send()?;

            let body: graphql_client::Response<open_pull_requests::ResponseData> = res.json()?;
            if let Some(data) = body.data {
                if let Some(repository) = data.repository {
                    if let Some(nodes) = repository.pull_requests.nodes {
                        prs.extend(nodes.into_iter().filter_map(|v| {
                            v.map(|node| PullRequest {
                                title: node.title,
                                number: node.number,
                                repo: repo.to_string(),
                                reviewers: node.review_requests.map_or(Vec::new(), |rr| {
                                    rr.edges.map_or(Vec::new(), |edges| {
                                        edges
                                            .into_iter()
                                            .filter_map(|edge| {
                                                edge.and_then(|edge| {
                                                    edge.node.and_then(|node| {
                                                        node.requested_reviewer
                                                            .and_then(|reviewer| match reviewer {
                                                                open_pull_requests::OpenPullRequestsRepositoryPullRequestsNodesReviewRequestsEdgesNodeRequestedReviewer::User(u) => Some(u.login),
                                                                _ => None,
                                                            })
                                                    })
                                                })
                                            })
                                            .collect()
                                    })
                                }),
                                assignees: node.assignees.nodes.map_or(Vec::new(), |nodes| {
                                    nodes
                                        .into_iter()
                                        .filter_map(|v| v.map(|node| node.login))
                                        .collect()
                                }),
                            })
                        }));
                    }
                }
            }
        }
        Ok(prs)
    }

    pub fn merged_pull_requests_per_login(
        &self,
        repos: &[String],
        since: &chrono::DateTime<chrono::Utc>,
    ) -> Result<HashMap<String, (usize, i64)>> {
        let mut prs = HashMap::new();
        for repo in repos {
            let (owner, name) = repo
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("Invalid repository format: {}", repo))?;
            let query = MergedPullRequests::build_query(merged_pull_requests::Variables {
                owner: owner.to_string(),
                name: name.to_string(),
            });
            let res = self
                .inner
                .post("https://api.github.com/graphql")
                .bearer_auth(&self.token[owner])
                .json(&query)
                .send()?;

            let body: graphql_client::Response<merged_pull_requests::ResponseData> = res.json()?;
            if let Some(data) = body.data {
                if let Some(repository) = data.repository {
                    if let Some(nodes) = repository.pull_requests.nodes {
                        for node in nodes.into_iter().flatten() {
                            let login = if let Some(author) = node.author {
                                author.login
                            } else {
                                continue;
                            };
                            let created_at =
                                chrono::DateTime::parse_from_rfc3339(&node.created_at)?;
                            if created_at < *since {
                                break;
                            }
                            let count = prs.entry(login).or_insert((0, 0));
                            count.0 += 1;
                            count.1 += node.comments.total_count;
                        }
                    }
                }
            }
        }
        Ok(prs)
    }
}

#[derive(Debug)]
pub struct Issue {
    pub title: String,
    pub number: i64,
    pub repo: String,
    pub assignees: Vec<String>,
}

#[derive(Debug)]
pub struct IssueMetadata {
    pub author: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub created_at: chrono::DateTime<chrono::offset::FixedOffset>,
    pub closed_at: Option<chrono::DateTime<chrono::offset::FixedOffset>>,
}

#[derive(Debug)]
pub struct PullRequest {
    pub title: String,
    pub number: i64,
    pub repo: String,
    pub reviewers: Vec<String>,
    pub assignees: Vec<String>,
}
