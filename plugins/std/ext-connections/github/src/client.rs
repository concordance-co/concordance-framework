use std::time::Duration;

use crate::log;
use crate::plugin::injector::error::HttpError;
use crate::plugin::injector::error::PluginError;
use crate::plugin::injector::host::Level;
use roctogen::api::{issues, orgs, pulls, repos, search, users};
use roctogen::models::CommitComparison;
use roctogen::models::{
    PostIssuesCreate, PostIssuesCreateComment, PostPullsCreate, PostReposCreateRelease,
};
use roctokit::adapters::AdapterError;
use roctokit::adapters::Client;
use roctokit::adapters::GitHubRequest;
use roctokit::adapters::GitHubResponseExt;
use roctokit::auth::Auth;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wstd::runtime::block_on;

// Helper function to convert serde_json::Value to Plugin Error
impl From<AdapterError> for PluginError {
    fn from(err: AdapterError) -> Self {
        PluginError::Unexpected(format!("Github API Error: {}", err))
    }
}

/// A client for the GitHub API
pub struct InnerGitHubClient {
    pub auth: Auth,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitHubOperation {
    /// Gets information about the authenticated user
    #[serde(rename = "current_user")]
    #[default]
    CurrentUser,

    /// Gets information about a specific GitHub user
    #[serde(rename = "get_user")]
    GetUser { username: String },

    /// Lists repositories for a user - will not work for organizations
    #[serde(rename = "list_user_repos")]
    ListUserRepos {
        username: String,
        per_page: Option<u16>,
        page: Option<u16>,
    },

    /// Gets a specific repository
    #[serde(rename = "get_repo")]
    GetRepo { owner: String, repo: String },

    /// Lists issues for a repository
    #[serde(rename = "list_issues")]
    ListIssues {
        owner: String,
        repo: String,
        state: Option<String>,
        labels: Option<Vec<String>>,
        since: Option<String>,
        per_page: Option<u16>,
        page: Option<u16>,
    },

    /// Gets a specific issue
    #[serde(rename = "get_issue")]
    GetIssue {
        owner: String,
        repo: String,
        issue_number: u64,
    },

    /// Creates a new issue
    #[serde(rename = "create_issue")]
    CreateIssue {
        owner: String,
        repo: String,
        title: String,
        body: Option<String>,
        assignees: Option<Vec<String>>,
        labels: Option<Vec<String>>,
    },

    /// Lists pull requests for a repository
    #[serde(rename = "list_pulls")]
    ListPulls {
        owner: String,
        repo: String,
        state: Option<String>,
        head: Option<String>,
        base: Option<String>,
        per_page: Option<u16>,
        page: Option<u16>,
    },

    /// Gets a specific pull request
    #[serde(rename = "get_pull")]
    GetPull {
        owner: String,
        repo: String,
        pull_number: u64,
    },

    /// Creates a new pull request
    #[serde(rename = "create_pull")]
    CreatePull {
        owner: String,
        repo: String,
        title: String,
        body: Option<String>,
        head: String,
        base: String,
        draft: Option<bool>,
    },

    /// Lists releases for a repository
    #[serde(rename = "list_releases")]
    ListReleases {
        owner: String,
        repo: String,
        per_page: Option<u16>,
        page: Option<u16>,
    },

    /// Creates a new release
    #[serde(rename = "create_release")]
    CreateRelease {
        owner: String,
        repo: String,
        tag_name: String,
        name: Option<String>,
        body: Option<String>,
        draft: Option<bool>,
        prerelease: Option<bool>,
    },

    /// Creates a new comment on an issue
    #[serde(rename = "create_issue_comment")]
    CreateIssueComment {
        owner: String,
        repo: String,
        issue_number: u64,
        body: String,
    },

    /// Gets a specific comment
    #[serde(rename = "get_comment")]
    GetComment {
        owner: String,
        repo: String,
        comment_id: u64,
    },

    /// Lists comments for a specific issue
    #[serde(rename = "list_comments")]
    ListComments {
        owner: String,
        repo: String,
        issue_number: u64,
        per_page: Option<u16>,
        page: Option<u16>,
    },

    /// Gets a specific commit
    #[serde(rename = "get_commit")]
    GetCommit {
        owner: String,
        repo: String,
        sha: String,
    },

    /// Lists commits for a repository
    #[serde(rename = "list_commits")]
    ListCommits {
        owner: String,
        repo: String,
        path: Option<String>,
        author: Option<String>,
        since: Option<String>,
        until: Option<String>,
        per_page: Option<u16>,
        page: Option<u16>,
    },

    /// Searches repositories
    #[serde(rename = "search_repos")]
    SearchRepos {
        query: String,
        sort: Option<String>,
        order: Option<String>,
        per_page: Option<u16>,
        page: Option<u16>,
    },

    /// Searches issues
    #[serde(rename = "search_issues")]
    SearchIssues {
        query: String,
        sort: Option<String>,
        order: Option<String>,
        per_page: Option<u16>,
        page: Option<u16>,
    },

    /// Searches users
    #[serde(rename = "search_users")]
    SearchUsers {
        query: String,
        sort: Option<String>,
        order: Option<String>,
        per_page: Option<u16>,
        page: Option<u16>,
    },

    /// Lists all repositories for an organization - will not work for users
    #[serde(rename = "list_organization_repos")]
    ListOrganizationRepos {
        organization: String,
        r#type: Option<String>,
        sort: Option<String>,
        direction: Option<String>,
        per_page: Option<u16>,
        page: Option<u16>,
    },

    /// Lists all members of an organization
    #[serde(rename = "list_organization_members")]
    ListOrganizationMembers {
        organization: String,
        per_page: Option<u16>,
        page: Option<u16>,
    },
}

// WakiClient provides an HTTP client service for Octocrab using Waki
pub struct WakiClient {
    // No need for an Arc<client> since waki::Client is already lightweight
    pub auth: Auth,
}

impl WakiClient {
    // Create a new WakiClient
    pub fn new(auth: Auth) -> Self {
        Self { auth }
    }

    pub fn fetch(&self, req: waki::RequestBuilder) -> Result<WakiResponse, PluginError> {
        let resp = req
            .send()
            .map_err(|e| PluginError::Http(HttpError::BadStatus(e.to_string())))?;
        let resp = WakiResponse {
            status_code: resp.status_code(),
            body: resp
                .body()
                .map_err(|_| PluginError::Http(HttpError::InvalidResponse))?,
        };
        log(
            Level::Info,
            &format!(
                "GitHub API Response: {}, {}",
                resp.status_code,
                String::from_utf8_lossy(&resp.body[..])
            ),
        );
        Ok(resp)
    }
}

impl roctokit::adapters::Client for WakiClient {
    type Req = waki::RequestBuilder;
    type Err = PluginError;
    type Body = Vec<u8>;

    fn new(auth: &Auth) -> Result<Self, Self::Err> {
        Ok(WakiClient::new(auth.clone()))
    }

    fn fetch(
        &self,
        req: Self::Req,
    ) -> Result<impl roctokit::adapters::GitHubResponseExt, Self::Err> {
        self.fetch(req)
    }

    async fn fetch_async(
        &self,
        request: Self::Req,
    ) -> Result<impl roctokit::adapters::GitHubResponseExt, Self::Err> {
        self.fetch(request)
    }

    fn build(
        &self,
        req: roctokit::adapters::GitHubRequest<Self::Body>,
    ) -> Result<Self::Req, Self::Err> {
        // Set method
        let method = match req.method {
            "GET" => waki::Method::Get,
            "POST" => waki::Method::Post,
            "PUT" => waki::Method::Put,
            "DELETE" => waki::Method::Delete,
            "PATCH" => waki::Method::Patch,
            _ => {
                return Err(PluginError::Unexpected(format!(
                    "Unsupported method: {}",
                    req.method
                )));
            }
        };

        let mut request_builder = waki::RequestBuilder::new(method.clone(), &req.uri);

        // Add common headers
        request_builder = request_builder
            .header("Accept", "application/vnd.github.v3+json")
            .header("User-Agent", "nullborn/industries")
            .header("Content-Type", "application/json")
            .header("X-GitHub-Api-Version", "2022-11-28");

        // Add custom headers
        let headers = req
            .headers
            .iter()
            .map(|(hk, hv)| (hk.to_string(), hv.to_string()))
            .collect::<Vec<_>>();
        for (hk, hv) in headers {
            request_builder = request_builder.header(&*Box::leak(hk.into_boxed_str()), hv);
        }

        // Add auth
        match &self.auth {
            Auth::Basic { user, pass } => {
                use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};
                let creds = format!("{}:{}", user, pass);
                request_builder = request_builder.header(
                    "Authorization",
                    format!("Basic {}", BASE64_STANDARD.encode(creds.as_bytes())),
                );
            }
            Auth::Token(token) => {
                request_builder =
                    request_builder.header("Authorization", format!("token {}", token));
            }
            Auth::Bearer(bearer) => {
                request_builder =
                    request_builder.header("Authorization", format!("Bearer {}", bearer));
            }
            Auth::None => {
                log(Level::Warn, "No authentication provided");
            }
        }

        // Add body if present
        if let Some(ref body) = req.body {
            request_builder = request_builder.body(body.clone());
        }

        // Log the request for debugging
        log(
            Level::Info,
            &format!("GitHub API Request: {} {}", req.method, req.uri),
        );

        request_builder = request_builder.connect_timeout(Duration::from_secs(30));

        Ok(request_builder)
    }

    fn from_json<E: serde::Serialize>(model: E) -> Result<Self::Body, Self::Err> {
        Ok(serde_json::to_vec(&model)?)
    }
}

// Response adapter for Waki
pub struct WakiResponse {
    body: Vec<u8>,
    status_code: u16,
}

impl roctokit::adapters::GitHubResponseExt for WakiResponse {
    fn status_code(&self) -> u16 {
        self.status_code
    }

    fn is_success(&self) -> bool {
        (200..300).contains(&self.status_code)
    }

    fn to_json<E>(self) -> Result<E, serde_json::Error>
    where
        E: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        serde_json::from_slice(&self.body)
    }

    async fn to_json_async<E>(self) -> Result<E, serde_json::Error>
    where
        E: serde::de::DeserializeOwned + std::fmt::Debug + std::marker::Unpin,
    {
        self.to_json()
    }
}

impl From<PluginError> for AdapterError {
    fn from(err: PluginError) -> Self {
        AdapterError::Client {
            description: format!("Plugin Error: {}", err),
            source: None,
        }
    }
}

// Helper function to create a client with authentication
fn client(auth: &Auth) -> WakiClient {
    WakiClient::new(auth.clone())
}

impl InnerGitHubClient {
    /// Creates a new GitHub client
    #[allow(dead_code)]
    pub fn new(auth: Auth) -> Self {
        Self { auth }
    }

    /// Creates a new GitHub client with token authentication
    pub fn with_token(token: String) -> Self {
        Self {
            auth: Auth::Token(token),
        }
    }

    /// Creates a new GitHub client with bearer authentication
    #[allow(dead_code)]
    pub fn with_bearer(token: String) -> Self {
        Self {
            auth: Auth::Bearer(token),
        }
    }

    /// Creates a new GitHub client with basic authentication
    #[allow(dead_code)]
    pub fn with_basic(username: String, password: String) -> Self {
        Self {
            auth: Auth::Basic {
                user: username,
                pass: password,
            },
        }
    }

    /// Creates a new GitHub client with no authentication
    #[allow(dead_code)]
    pub fn anonymous() -> Self {
        Self { auth: Auth::None }
    }

    /// Executes a GitHub operation and returns the result as JSON
    pub fn execute(&self, op: GitHubOperation) -> Result<Value, PluginError> {
        let client = client(&self.auth);

        block_on(async move {
            match op {
                GitHubOperation::CurrentUser => {
                    let users_client = users::new(&client);
                    let response = users_client.get_authenticated_async().await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::GetUser { username } => {
                    let users_client = users::new(&client);
                    let response = users_client.get_by_username_async(&username).await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::ListUserRepos {
                    username,
                    per_page,
                    page,
                } => {
                    let repos_client = repos::new(&client);
                    let mut params = repos::ReposListForUserParams::new();

                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let response = repos_client
                        .list_for_user_async(&username, Some(params))
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::GetRepo { owner, repo } => {
                    let repos_client = repos::new(&client);
                    let response = repos_client.get_async(&owner, &repo).await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::ListIssues {
                    owner,
                    repo,
                    state,
                    labels,
                    since,
                    per_page,
                    page,
                } => {
                    let issues_client = issues::new(&client);
                    let mut params = issues::IssuesListForRepoParams::new();

                    if let Some(ref state_val) = state {
                        params = params.state(state_val);
                    }

                    let label_string = if let Some(ref labels) = labels {
                        labels.join(",")
                    } else {
                        "".to_string()
                    };
                    params = params.labels(&label_string);
                    if let Some(since) = since {
                        // Parse the since string to DateTime
                        use chrono::DateTime;
                        use chrono::Utc;
                        let since_date = DateTime::parse_from_rfc3339(&since)
                            .map_err(|e| {
                                PluginError::Unexpected(format!("Invalid date format: {}", e))
                            })?
                            .with_timezone(&Utc);
                        params = params.since(since_date);
                    }
                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let mut response = issues_client
                        .list_for_repo_async(&owner, &repo, Some(params))
                        .await?;
                    response.retain(|response| response.pull_request.is_none());
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::GetIssue {
                    owner,
                    repo,
                    issue_number,
                } => {
                    let issues_client = issues::new(&client);
                    let response = issues_client
                        .get_async(&owner, &repo, issue_number as i32)
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::CreateIssue {
                    owner,
                    repo,
                    title,
                    body,
                    assignees,
                    labels,
                } => {
                    let issues_client = issues::new(&client);
                    let create_issue = PostIssuesCreate {
                        title: Some(title.into()),
                        body,
                        assignees,
                        // Convert Vec<String> to Vec<OneOfbody164LabelsItems> if labels is Some
                        labels: labels.map(|l| l.into_iter().map(|label| label.into()).collect()),
                        ..Default::default()
                    };

                    let response = issues_client
                        .create_async(&owner, &repo, create_issue)
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::ListPulls {
                    owner,
                    repo,
                    state,
                    head,
                    base,
                    per_page,
                    page,
                } => {
                    let pulls_client = pulls::new(&client);
                    let mut params = pulls::PullsListParams::new();

                    if let Some(ref state_val) = state {
                        params = params.state(state_val);
                    }
                    if let Some(ref head_val) = head {
                        params = params.head(head_val);
                    }
                    if let Some(ref base_val) = base {
                        params = params.base(base_val);
                    }
                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let response = pulls_client.list_async(&owner, &repo, Some(params)).await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::GetPull {
                    owner,
                    repo,
                    pull_number,
                } => {
                    let pulls_client = pulls::new(&client);
                    let response = pulls_client
                        .get_async(&owner, &repo, pull_number as i32)
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::CreatePull {
                    owner,
                    repo,
                    title,
                    body,
                    head,
                    base,
                    draft,
                } => {
                    let pulls_client = pulls::new(&client);
                    let create_pull = PostPullsCreate {
                        title: Some(title),
                        head: Some(head),
                        base: Some(base),
                        body,
                        draft,
                        ..Default::default()
                    };

                    let response = pulls_client
                        .create_async(&owner, &repo, create_pull)
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::ListReleases {
                    owner,
                    repo,
                    per_page,
                    page,
                } => {
                    let repos_client = repos::new(&client);
                    let mut params = repos::ReposListReleasesParams::new();

                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let response = repos_client
                        .list_releases_async(&owner, &repo, Some(params))
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::CreateRelease {
                    owner,
                    repo,
                    tag_name,
                    name,
                    body,
                    draft,
                    prerelease,
                } => {
                    let repos_client = repos::new(&client);
                    let create_release = PostReposCreateRelease {
                        tag_name: Some(tag_name),
                        name,
                        body,
                        draft,
                        prerelease,
                        ..Default::default()
                    };

                    let response = repos_client
                        .create_release_async(&owner, &repo, create_release)
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::CreateIssueComment {
                    owner,
                    repo,
                    issue_number,
                    body,
                } => {
                    let issues_client = issues::new(&client);
                    let create_comment = PostIssuesCreateComment { body: Some(body) };

                    let response = issues_client
                        .create_comment_async(&owner, &repo, issue_number as i32, create_comment)
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::GetComment {
                    owner,
                    repo,
                    comment_id,
                } => {
                    let issues_client = issues::new(&client);
                    let response = issues_client
                        .get_comment_async(&owner, &repo, comment_id as i64)
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::ListComments {
                    owner,
                    repo,
                    issue_number,
                    per_page,
                    page,
                } => {
                    let issues_client = issues::new(&client);
                    let mut params = issues::IssuesListCommentsParams::new();

                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let response = issues_client
                        .list_comments_async(&owner, &repo, issue_number as i32, Some(params))
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::GetCommit { owner, repo, sha } => {
                    let request_uri = format!(
                        "{}/repos/{}/{}/commits/{}",
                        crate::GITHUB_BASE_URL,
                        owner,
                        repo,
                        sha
                    );
                    let req = GitHubRequest {
                        uri: request_uri,
                        body: None::<<WakiClient as Client>::Body>,
                        method: "GET",
                        headers: vec![],
                    };
                    let req = client.build(req)?;
                    let github_response: WakiResponse = client.fetch(req)?;
                    let response: CommitComparison = github_response.to_json()?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::ListCommits {
                    owner,
                    repo,
                    path,
                    author,
                    since,
                    until,
                    per_page,
                    page,
                } => {
                    let repos_client = repos::new(&client);
                    let mut params = repos::ReposListCommitsParams::new();

                    if let Some(ref path_val) = path {
                        params = params.path(path_val);
                    }
                    if let Some(ref author_val) = author {
                        params = params.author(author_val);
                    }
                    if let Some(since) = since {
                        // Parse the since string to DateTime
                        use chrono::DateTime;
                        use chrono::Utc;
                        let since_date = DateTime::parse_from_rfc3339(&since)
                            .map_err(|e| {
                                PluginError::Unexpected(format!("Invalid date format: {}", e))
                            })?
                            .with_timezone(&Utc);
                        params = params.since(since_date);
                    }
                    if let Some(until) = until {
                        // Parse the until string to DateTime
                        use chrono::DateTime;
                        use chrono::Utc;
                        let until_date = DateTime::parse_from_rfc3339(&until)
                            .map_err(|e| {
                                PluginError::Unexpected(format!("Invalid date format: {}", e))
                            })?
                            .with_timezone(&Utc);
                        params = params.until(until_date);
                    }
                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let response = repos_client
                        .list_commits_async(&owner, &repo, Some(params))
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::SearchRepos {
                    query,
                    sort,
                    order,
                    per_page,
                    page,
                } => {
                    let search_client = search::new(&client);
                    let mut params = search::SearchReposParams::new().q(&query);

                    if let Some(ref sort_val) = sort {
                        params = params.sort(sort_val);
                    }
                    if let Some(ref order_val) = order {
                        params = params.order(order_val);
                    }
                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let response = search_client.repos_async(params).await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::SearchIssues {
                    query,
                    sort,
                    order,
                    per_page,
                    page,
                } => {
                    let search_client = search::new(&client);
                    let mut params = search::SearchCodeParams::new().q(&query);

                    if let Some(ref sort_val) = sort {
                        params = params.sort(sort_val);
                    }
                    if let Some(ref order_val) = order {
                        params = params.order(order_val);
                    }
                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let response = search_client.code_async(params).await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::SearchUsers {
                    query,
                    sort,
                    order,
                    per_page,
                    page,
                } => {
                    let search_client = search::new(&client);
                    let mut params = search::SearchUsersParams::new().q(&query);

                    if let Some(ref sort_val) = sort {
                        params = params.sort(sort_val);
                    }
                    if let Some(ref order_val) = order {
                        params = params.order(order_val);
                    }
                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let response = search_client.users_async(params).await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::ListOrganizationRepos {
                    organization,
                    r#type,
                    sort,
                    direction,
                    per_page,
                    page,
                } => {
                    let repos_client = repos::new(&client);
                    let mut params = repos::ReposListForOrgParams::new();

                    if let Some(ref type_val) = r#type {
                        params = params._type(type_val);
                    }
                    if let Some(ref sort) = sort {
                        params = params.sort(sort);
                    }
                    if let Some(ref direction) = direction {
                        params = params.direction(direction);
                    }
                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let response = repos_client
                        .list_for_org_async(&organization, Some(params))
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
                GitHubOperation::ListOrganizationMembers {
                    organization,
                    per_page,
                    page,
                } => {
                    let orgs_client = orgs::new(&client);
                    let mut params = orgs::OrgsListMembersParams::new();

                    if let Some(per_page) = per_page {
                        params = params.per_page(per_page);
                    }
                    if let Some(page) = page {
                        params = params.page(page);
                    }

                    let response = orgs_client
                        .list_members_async(&organization, Some(params))
                        .await?;
                    serde_json::to_value(response).map_err(PluginError::from)
                }
            }
        })
    }
}
