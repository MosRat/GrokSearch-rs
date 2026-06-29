use async_trait::async_trait;
use base64::Engine;
use reqwest::header::{AUTHORIZATION, USER_AGENT};
use reqwest::Client;
use url::Url;

use crate::sources::{get_json, SourceCaps, SourceExtractor, SourceType};
use grok_search_types::{
    GrokSearchError, RepoKind, RepoLinks, RepoMetadataOutput, RepoProvider, RepoText, Result,
};

const UA: &str = "grok-search-rs/0.1 (https://github.com/MosRat/GrokSearch-rs)";

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GithubRaw {
    pub title: String,
    pub state: String,
    pub merged: Option<bool>,
    pub author: String,
    pub body: String,
    pub labels: Vec<String>,
    pub comments: Vec<GithubComment>,
    pub is_pr: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GithubComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GithubRepoRaw {
    pub full_name: String,
    pub description: Option<String>,
    pub default_branch: String,
    pub stars: u64,
    pub forks: u64,
    pub license: Option<String>,
    pub readme: String,
}

pub struct GithubIssueExtractor {
    pub token: Option<String>,
}

pub struct GithubPrExtractor {
    pub token: Option<String>,
}

pub struct GithubRepoExtractor {
    pub token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRepoLocator {
    pub owner: String,
    pub repo: String,
}

fn matches_github(url: &Url, segment_kind: &str) -> bool {
    if url.host_str() != Some("github.com") {
        return false;
    }
    let segs: Vec<&str> = match url.path_segments() {
        Some(it) => it.filter(|s| !s.is_empty()).collect(),
        None => return false,
    };
    segs.len() == 4 && segs[2] == segment_kind && segs[3].parse::<u64>().is_ok()
}

fn matches_github_repo(url: &Url) -> bool {
    if url.host_str() != Some("github.com") {
        return false;
    }
    let segs: Vec<&str> = match url.path_segments() {
        Some(it) => it.filter(|s| !s.is_empty()).collect(),
        None => return false,
    };
    segs.len() == 2
}

pub fn parse_repo_url(url: &Url) -> Result<GithubRepoLocator> {
    if url.host_str() != Some("github.com") {
        return Err(GrokSearchError::InvalidParams(
            "github repo URL must use github.com".into(),
        ));
    }
    let segs: Vec<String> = url
        .path_segments()
        .map(|it| it.filter(|s| !s.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    if segs.len() != 2 {
        return Err(GrokSearchError::InvalidParams(
            "github repo URL must be https://github.com/{owner}/{repo}".into(),
        ));
    }
    Ok(GithubRepoLocator {
        owner: segs[0].clone(),
        repo: segs[1].clone(),
    })
}

pub fn repo_locator(owner: &str, repo: &str) -> Result<GithubRepoLocator> {
    let owner = owner.trim();
    let repo = repo.trim();
    if owner.is_empty() || repo.is_empty() {
        return Err(GrokSearchError::InvalidParams(
            "github owner and name are required".into(),
        ));
    }
    if owner.contains('/') || repo.contains('/') {
        return Err(GrokSearchError::InvalidParams(
            "github owner and name must not contain '/'".into(),
        ));
    }
    Ok(GithubRepoLocator {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

/// Page size for any comment list. `/comments` endpoints default to 30 results
/// per page, which silently drops later comments and prevents the renderer's
/// "more comments" fold from ever firing. Request `max_comments + 1` so the
/// renderer can both show `max_comments` and detect there are more. GitHub caps
/// `per_page` at 100; callers needing more than that would require true page
/// iteration (out of scope — `source_max_comments` defaults to 30).
fn per_page(max_comments: usize) -> usize {
    max_comments.saturating_add(1).min(100)
}

/// Conversation (issue) comments — present on both issues and PRs.
fn comments_url(owner: &str, repo: &str, number: &str, max_comments: usize) -> String {
    format!(
        "https://api.github.com/repos/{owner}/{repo}/issues/{number}/comments?per_page={}",
        per_page(max_comments)
    )
}

/// Inline PR review comments (code-review threads). Distinct from conversation
/// comments and often where the actionable discussion lives.
fn pr_review_comments_url(owner: &str, repo: &str, number: &str, max_comments: usize) -> String {
    format!(
        "https://api.github.com/repos/{owner}/{repo}/pulls/{number}/comments?per_page={}",
        per_page(max_comments)
    )
}

/// PR review summaries (APPROVE / REQUEST_CHANGES / COMMENT bodies).
fn pr_reviews_url(owner: &str, repo: &str, number: &str, max_comments: usize) -> String {
    format!(
        "https://api.github.com/repos/{owner}/{repo}/pulls/{number}/reviews?per_page={}",
        per_page(max_comments)
    )
}

fn str_field(v: &serde_json::Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn login(v: &serde_json::Value) -> String {
    v.get("user")
        .and_then(|u| u.get("login"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// Map a `[...comments...]` array (issue or inline review comments) to
/// `GithubComment`s. Both shapes expose `user.login`, `body`, `created_at`.
fn parse_comments(json: &serde_json::Value) -> Vec<GithubComment> {
    json.as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| GithubComment {
                    author: login(c),
                    body: str_field(c, "body"),
                    created_at: str_field(c, "created_at"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Map a `[...reviews...]` array to `GithubComment`s, keeping only reviews that
/// carry a body (an APPROVE with no text adds no evidence). Reviews timestamp
/// with `submitted_at` rather than `created_at`.
fn parse_review_bodies(json: &serde_json::Value) -> Vec<GithubComment> {
    json.as_array()
        .map(|arr| {
            arr.iter()
                .map(|r| GithubComment {
                    author: login(r),
                    body: str_field(r, "body"),
                    created_at: str_field(r, "submitted_at"),
                })
                .filter(|c| !c.body.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) async fn fetch(
    client: &Client,
    url: &Url,
    token: Option<&str>,
    is_pr: bool,
    max_comments: usize,
) -> Result<GithubRaw> {
    let segs: Vec<String> = url
        .path_segments()
        .map(|it| it.filter(|s| !s.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    if segs.len() < 4 {
        return Err(GrokSearchError::Parse(
            "github: unexpected URL shape".into(),
        ));
    }
    let (owner, repo, number) = (&segs[0], &segs[1], &segs[3]);

    let auth = token.map(|t| format!("Bearer {t}"));
    let mut headers: Vec<(reqwest::header::HeaderName, &str)> = vec![(USER_AGENT, UA)];
    if let Some(ref a) = auth {
        headers.push((AUTHORIZATION, a.as_str()));
    }

    let main_url = if is_pr {
        format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}")
    } else {
        format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}")
    };
    let comments_url = comments_url(owner, repo, number, max_comments);

    // For PRs, the conversation thread (`/issues/{n}/comments`) omits inline
    // code-review comments and review summaries — usually the actionable
    // feedback. Fetch those two extra endpoints concurrently and merge them.
    // They are best-effort: a failure degrades to empty rather than failing the
    // whole specialist (so a PR still renders its body + conversation comments).
    let (main, comments) = if is_pr {
        let review_comments_url = pr_review_comments_url(owner, repo, number, max_comments);
        let reviews_url = pr_reviews_url(owner, repo, number, max_comments);
        let (main_res, conv_res, review_res, reviews_res) = tokio::join!(
            get_json(client, &main_url, &headers, "github"),
            get_json(client, &comments_url, &headers, "github_comments"),
            get_json(
                client,
                &review_comments_url,
                &headers,
                "github_review_comments"
            ),
            get_json(client, &reviews_url, &headers, "github_reviews"),
        );
        let main = main_res?;
        let mut comments = parse_comments(&conv_res?);
        if let Ok(json) = review_res {
            comments.extend(parse_comments(&json));
        }
        if let Ok(json) = reviews_res {
            comments.extend(parse_review_bodies(&json));
        }
        // ISO-8601 timestamps sort lexicographically = chronologically.
        comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        (main, comments)
    } else {
        let (main_res, conv_res) = tokio::join!(
            get_json(client, &main_url, &headers, "github"),
            get_json(client, &comments_url, &headers, "github_comments"),
        );
        (main_res?, parse_comments(&conv_res?))
    };

    let labels = main
        .get("labels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(GithubRaw {
        title: str_field(&main, "title"),
        state: str_field(&main, "state"),
        merged: main.get("merged").and_then(|v| v.as_bool()),
        author: main
            .get("user")
            .and_then(|u| u.get("login"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        body: str_field(&main, "body"),
        labels,
        comments,
        is_pr,
    })
}

pub(crate) async fn fetch_repo(
    client: &Client,
    url: &Url,
    token: Option<&str>,
) -> Result<GithubRepoRaw> {
    let locator = parse_repo_url(url)
        .map_err(|_| GrokSearchError::Parse("github repo: unexpected URL shape".into()))?;
    let owner = &locator.owner;
    let repo = &locator.repo;
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut headers: Vec<(reqwest::header::HeaderName, &str)> = vec![(USER_AGENT, UA)];
    if let Some(ref a) = auth {
        headers.push((AUTHORIZATION, a.as_str()));
    }

    let repo_url = format!("https://api.github.com/repos/{owner}/{repo}");
    let readme_url = format!("https://api.github.com/repos/{owner}/{repo}/readme");
    let (repo_json, readme_json) = tokio::join!(
        get_json(client, &repo_url, &headers, "github_repo"),
        get_json(client, &readme_url, &headers, "github_readme"),
    );
    let repo_json = repo_json?;
    let readme_json = readme_json?;
    let readme = decode_readme(&readme_json)?;
    if readme.trim().is_empty() {
        return Err(GrokSearchError::Parse("github readme empty".into()));
    }
    let meta = parse_repo_raw(&repo_json);
    Ok(GithubRepoRaw { readme, ..meta })
}

pub async fn fetch_repo_metadata(
    client: &Client,
    locator: &GithubRepoLocator,
    token: Option<&str>,
    include_readme: bool,
    max_text_chars: Option<usize>,
) -> Result<RepoMetadataOutput> {
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut headers: Vec<(reqwest::header::HeaderName, &str)> = vec![(USER_AGENT, UA)];
    if let Some(ref a) = auth {
        headers.push((AUTHORIZATION, a.as_str()));
    }

    let repo_url = format!(
        "https://api.github.com/repos/{}/{}",
        locator.owner, locator.repo
    );
    let repo_json = get_json(client, &repo_url, &headers, "github_repo").await?;
    let raw = parse_repo_raw(&repo_json);
    let html = format!("https://github.com/{}", raw.full_name);
    let mut warnings = Vec::new();
    let mut readme = None;
    let readme_api = format!(
        "https://api.github.com/repos/{}/{}/readme",
        locator.owner, locator.repo
    );

    if include_readme {
        match get_json(client, &readme_api, &headers, "github_readme").await {
            Ok(json) => match decode_readme(&json) {
                Ok(text) if !text.trim().is_empty() => {
                    readme = Some(limit_repo_text(text, max_text_chars));
                }
                Ok(_) => warnings.push("github readme empty".to_string()),
                Err(err) => warnings.push(format!("github readme skipped: {err}")),
            },
            Err(err) => warnings.push(format!("github readme skipped: {err}")),
        }
    }

    Ok(RepoMetadataOutput {
        provider: RepoProvider::Github,
        kind: RepoKind::GithubRepository,
        id: raw.full_name.clone(),
        name: locator.repo.clone(),
        owner: Some(locator.owner.clone()),
        description: raw.description,
        license: raw.license,
        tags: Vec::new(),
        stars: Some(raw.stars),
        forks: Some(raw.forks),
        downloads: None,
        likes: None,
        created_at: repo_json
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        updated_at: repo_json
            .get("updated_at")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        default_branch: Some(raw.default_branch),
        links: RepoLinks {
            html,
            api: repo_url,
            readme: include_readme.then_some(readme_api),
            card: None,
        },
        readme,
        card: None,
        warnings,
    })
}

fn decode_readme(json: &serde_json::Value) -> Result<String> {
    let content = json
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GrokSearchError::Parse("github readme missing content".into()))?;
    let encoding = json
        .get("encoding")
        .and_then(|v| v.as_str())
        .unwrap_or("base64");
    if encoding != "base64" {
        return Err(GrokSearchError::Parse(format!(
            "github readme unsupported encoding: {encoding}"
        )));
    }
    let compact: String = content.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(compact)
        .map_err(|err| GrokSearchError::Parse(format!("github readme base64 decode: {err}")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_repo_raw(repo_json: &serde_json::Value) -> GithubRepoRaw {
    GithubRepoRaw {
        full_name: str_field(repo_json, "full_name"),
        description: repo_json
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        default_branch: str_field(repo_json, "default_branch"),
        stars: repo_json
            .get("stargazers_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        forks: repo_json
            .get("forks_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        license: repo_json
            .pointer("/license/spdx_id")
            .and_then(|v| v.as_str())
            .filter(|v| *v != "NOASSERTION")
            .map(str::to_string),
        readme: String::new(),
    }
}

pub(crate) fn limit_repo_text(mut content: String, max_chars: Option<usize>) -> RepoText {
    let Some(limit) = max_chars else {
        let original_length = content.chars().count();
        return RepoText {
            content,
            original_length,
            truncated: false,
        };
    };

    let mut count = 0usize;
    let mut cutoff = None;
    for (byte_idx, _) in content.char_indices() {
        if count == limit {
            cutoff = Some(byte_idx);
            break;
        }
        count += 1;
    }

    match cutoff {
        Some(byte_idx) => {
            let extra = content[byte_idx..].chars().count();
            content.truncate(byte_idx);
            RepoText {
                content,
                original_length: limit + extra,
                truncated: true,
            }
        }
        None => RepoText {
            content,
            original_length: count,
            truncated: false,
        },
    }
}

pub fn render(raw: &GithubRaw, caps: &SourceCaps) -> String {
    let mut out = format!("# {}\n\n", raw.title);
    let merged_suffix = if raw.is_pr {
        match raw.merged {
            Some(true) => " (merged)",
            _ if raw.state == "closed" => " (closed, not merged)",
            _ => "",
        }
    } else {
        ""
    };
    out.push_str(&format!("**State:** {}{}\n", raw.state, merged_suffix));
    out.push_str(&format!("**Author:** {}\n", raw.author));
    if !raw.labels.is_empty() {
        out.push_str(&format!("**Labels:** {}\n", raw.labels.join(", ")));
    }
    out.push_str(&format!("\n{}\n\n## Comments\n\n", raw.body));
    for c in raw.comments.iter().take(caps.max_comments) {
        out.push_str(&format!(
            "### Comment by {} ({})\n\n{}\n\n",
            c.author, c.created_at, c.body
        ));
    }
    if raw.comments.len() > caps.max_comments {
        let extra = raw.comments.len() - caps.max_comments;
        out.push_str(&format!("_还有 {extra} 条评论_\n"));
    }
    out
}

pub fn render_repo(raw: &GithubRepoRaw, _caps: &SourceCaps) -> String {
    let mut out = format!("# {}\n\n", raw.full_name);
    if let Some(description) = raw.description.as_deref().filter(|v| !v.trim().is_empty()) {
        out.push_str(description);
        out.push_str("\n\n");
    }
    out.push_str(&format!("**Default branch:** {}\n", raw.default_branch));
    out.push_str(&format!("**Stars:** {}\n", raw.stars));
    out.push_str(&format!("**Forks:** {}\n", raw.forks));
    if let Some(license) = raw.license.as_deref() {
        out.push_str(&format!("**License:** {license}\n"));
    }
    out.push_str("\n## README\n\n");
    out.push_str(raw.readme.trim());
    out.push('\n');
    out
}

#[async_trait]
impl SourceExtractor for GithubIssueExtractor {
    fn matches(&self, url: &Url) -> bool {
        matches_github(url, "issues")
    }
    fn kind(&self) -> SourceType {
        SourceType::GithubIssue
    }
    async fn fetch_render(&self, client: &Client, url: &Url, caps: &SourceCaps) -> Result<String> {
        let raw = fetch(client, url, self.token.as_deref(), false, caps.max_comments).await?;
        Ok(render(&raw, caps))
    }
}

#[async_trait]
impl SourceExtractor for GithubPrExtractor {
    fn matches(&self, url: &Url) -> bool {
        matches_github(url, "pull")
    }
    fn kind(&self) -> SourceType {
        SourceType::GithubPull
    }
    async fn fetch_render(&self, client: &Client, url: &Url, caps: &SourceCaps) -> Result<String> {
        let raw = fetch(client, url, self.token.as_deref(), true, caps.max_comments).await?;
        Ok(render(&raw, caps))
    }
}

#[async_trait]
impl SourceExtractor for GithubRepoExtractor {
    fn matches(&self, url: &Url) -> bool {
        matches_github_repo(url)
    }
    fn kind(&self) -> SourceType {
        SourceType::GithubRepo
    }
    async fn fetch_render(&self, client: &Client, url: &Url, caps: &SourceCaps) -> Result<String> {
        let raw = fetch_repo(client, url, self.token.as_deref()).await?;
        Ok(render_repo(&raw, caps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_url_requests_one_more_than_cap() {
        // +1 over max_comments lets render() detect "more comments" and fold.
        let u = comments_url("o", "r", "5", 30);
        assert_eq!(
            u,
            "https://api.github.com/repos/o/r/issues/5/comments?per_page=31"
        );
    }

    #[test]
    fn comments_url_clamps_per_page_to_github_max() {
        let u = comments_url("o", "r", "5", 250);
        assert!(u.ends_with("?per_page=100"), "got: {u}");
    }

    #[test]
    fn pr_endpoints_target_pulls_paths() {
        assert!(pr_review_comments_url("o", "r", "7", 30)
            .starts_with("https://api.github.com/repos/o/r/pulls/7/comments?per_page=31"));
        assert!(pr_reviews_url("o", "r", "7", 30)
            .starts_with("https://api.github.com/repos/o/r/pulls/7/reviews?per_page=31"));
    }

    #[test]
    fn parse_review_bodies_skips_empty_and_maps_submitted_at() {
        let json = serde_json::json!([
            { "user": { "login": "alice" }, "body": "needs changes", "submitted_at": "2024-01-02T00:00:00Z" },
            { "user": { "login": "bob" }, "body": "   ", "submitted_at": "2024-01-03T00:00:00Z" },
            { "user": { "login": "carol" }, "body": "", "submitted_at": "2024-01-04T00:00:00Z" }
        ]);
        let out = parse_review_bodies(&json);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].author, "alice");
        assert_eq!(out[0].created_at, "2024-01-02T00:00:00Z");
    }

    #[test]
    fn parse_comments_maps_user_login_and_timestamps() {
        let json = serde_json::json!([
            { "user": { "login": "dave" }, "body": "hi", "created_at": "2024-01-01T00:00:00Z" }
        ]);
        let out = parse_comments(&json);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].author, "dave");
        assert_eq!(out[0].body, "hi");
        assert_eq!(out[0].created_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn decode_readme_accepts_wrapped_base64() {
        let json = serde_json::json!({
            "encoding": "base64",
            "content": "IyBSZXBvCg==\n"
        });
        let out = decode_readme(&json).expect("readme");
        assert_eq!(out, "# Repo\n");
    }

    #[test]
    fn decode_readme_rejects_unknown_encoding() {
        let json = serde_json::json!({
            "encoding": "utf-8",
            "content": "# Repo\n"
        });
        let err = decode_readme(&json).expect_err("unknown encoding should fail");
        assert!(err.to_string().contains("unsupported encoding"));
    }

    #[test]
    fn parse_repo_url_accepts_owner_repo_only() {
        let url = Url::parse("https://github.com/owner/repo").unwrap();
        let locator = parse_repo_url(&url).unwrap();
        assert_eq!(locator.owner, "owner");
        assert_eq!(locator.repo, "repo");
    }

    #[test]
    fn parse_repo_url_rejects_deeper_paths() {
        let url = Url::parse("https://github.com/owner/repo/issues/1").unwrap();
        let err = parse_repo_url(&url).unwrap_err();
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn parse_repo_raw_maps_metadata() {
        let json = serde_json::json!({
            "full_name": "owner/repo",
            "description": "desc",
            "default_branch": "main",
            "stargazers_count": 42,
            "forks_count": 3,
            "license": { "spdx_id": "MIT" }
        });
        let raw = parse_repo_raw(&json);
        assert_eq!(raw.full_name, "owner/repo");
        assert_eq!(raw.license.as_deref(), Some("MIT"));
        assert_eq!(raw.stars, 42);
    }

    #[test]
    fn limit_repo_text_truncates_by_chars() {
        let text = limit_repo_text("abcdef".to_string(), Some(3));
        assert_eq!(text.content, "abc");
        assert_eq!(text.original_length, 6);
        assert!(text.truncated);
    }
}
