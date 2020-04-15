use anyhow::Result;
use graphql_client::GraphQLQuery;
use reqwest;
use std::collections::HashMap;

type DateTime = String;

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
    token: String,
    inner: reqwest::blocking::Client,
}

impl Client {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_string(),
            inner: reqwest::blocking::ClientBuilder::new()
                .user_agent(USER_AGENT)
                .build()
                .unwrap(),
        }
    }

    pub fn open_pull_requests(&self, repos: &[String]) -> Result<Vec<PullRequest>> {
        let mut prs = Vec::new();
        for repo in repos {
            let query = OpenPullRequests::build_query(open_pull_requests::Variables {
                owner: "petabi".to_string(),
                name: repo.to_string(),
            });
            let res = self
                .inner
                .post("https://api.github.com/graphql")
                .bearer_auth(&self.token)
                .json(&query)
                .send()?;

            let body: graphql_client::Response<open_pull_requests::ResponseData> = res.json()?;
            if let Some(data) = body.data {
                if let Some(repository) = data.repository {
                    if let Some(nodes) = repository.pull_requests.nodes {
                        prs.extend(nodes.into_iter().filter_map(|v| {
                            if let Some(node) = v {
                                Some(PullRequest {
                                    title: node.title,
                                    number: node.number,
                                    repo: repo.to_string(),
                                    assignees: if let Some(nodes) = node.assignees.nodes {
                                        nodes
                                            .into_iter()
                                            .filter_map(|v| {
                                                if let Some(node) = v {
                                                    Some(node.login)
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect()
                                    } else {
                                        Vec::new()
                                    },
                                })
                            } else {
                                None
                            }
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
            let query = MergedPullRequests::build_query(merged_pull_requests::Variables {
                owner: "petabi".to_string(),
                name: repo.to_string(),
            });
            let res = self
                .inner
                .post("https://api.github.com/graphql")
                .bearer_auth(&self.token)
                .json(&query)
                .send()?;

            let body: graphql_client::Response<merged_pull_requests::ResponseData> = res.json()?;
            if let Some(data) = body.data {
                if let Some(repository) = data.repository {
                    if let Some(nodes) = repository.pull_requests.nodes {
                        for node in nodes {
                            if let Some(node) = node {
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
                                let mut count = prs.entry(login).or_insert((0, 0));
                                count.0 += 1;
                                count.1 += node.comments.total_count;
                            }
                        }
                    }
                }
            }
        }
        Ok(prs)
    }
}

#[derive(Debug)]
pub struct PullRequest {
    pub title: String,
    pub number: i64,
    pub repo: String,
    pub assignees: Vec<String>,
}
