use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::error::{GhiError, Result};

pub async fn run_gh(args: &[String]) -> Result<Vec<u8>> {
    let mut cmd = Command::new("gh");
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => {
            return Err(GhiError::Gh(format!("gh {}: {e}", args.join(" "))));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim();
        let message = if message.is_empty() {
            "unknown error".to_string()
        } else {
            message.to_string()
        };
        return Err(GhiError::Gh(format!("gh {}: {}", args.join(" "), message)));
    }
    Ok(output.stdout)
}

pub fn cache_key_of(pr: &PullRequest) -> String {
    crate::cache::cache_key(&pr.url, pr.updated_at)
}

pub fn cache_key_of_updated(url: &str, updated_at: SystemTime) -> String {
    crate::cache::cache_key(url, updated_at)
}

pub type TeamsCache = std::sync::Arc<Mutex<Option<(Vec<Team>, Instant)>>>;
pub type RunnerFn = std::sync::Arc<dyn Fn(&[String]) -> Result<Vec<u8>> + Send + Sync>;

#[derive(Clone, Default)]
pub struct Gh {
    pub teams_cache: TeamsCache,
    runner: Option<RunnerFn>,
}

impl Gh {
    pub fn new() -> Self {
        Gh::default()
    }

    pub fn with_runner(runner: RunnerFn) -> Self {
        Gh {
            teams_cache: std::sync::Arc::new(Mutex::new(None)),
            runner: Some(runner),
        }
    }

    pub async fn run(&self, args: &[String]) -> Result<Vec<u8>> {
        if let Some(f) = &self.runner {
            return f(args);
        }
        run_gh(args).await
    }
}

pub async fn load_detail_and_diff(g: &Gh, pr: &PullRequest) -> Result<(PullRequestDetail, String)> {
    let p1 = pr.clone();
    let g2 = g.clone();
    let pr2 = pr.clone();
    let (detail_res, diff_res) =
        tokio::join!(async move { load_pr_detail(&g2, &p1).await }, async move {
            load_diff(g, &pr2).await
        });

    let diff = match diff_res {
        Ok(d) => d,
        Err(e) => {
            if is_pr_diff_too_large_error(&e) {
                "Diff omitted because GitHub reports this PR diff is too large to display."
                    .to_string()
            } else {
                return Err(e);
            }
        }
    };
    let detail = detail_res?;
    Ok((detail, diff))
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PullRequest {
    pub repository: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub updated_at: SystemTime,
    pub request: String,
    pub review_decision: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PullRequestDetail {
    pub base: PullRequest,
    pub body: String,
    pub created_at: SystemTime,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub merge_state_status: String,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub labels: Vec<String>,
    pub reviewers: Vec<ReviewSummary>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReviewSummary {
    pub author: String,
    pub state: String,
}

impl Default for PullRequest {
    fn default() -> Self {
        PullRequest {
            repository: String::new(),
            number: 0,
            title: String::new(),
            url: String::new(),
            author: String::new(),
            updated_at: SystemTime::UNIX_EPOCH,
            request: String::new(),
            review_decision: String::new(),
        }
    }
}

impl Default for PullRequestDetail {
    fn default() -> Self {
        PullRequestDetail {
            base: PullRequest::default(),
            body: String::new(),
            created_at: SystemTime::UNIX_EPOCH,
            base_ref_name: String::new(),
            head_ref_name: String::new(),
            merge_state_status: String::new(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            labels: Vec::new(),
            reviewers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Team {
    pub organization: String,
    pub slug: String,
}

pub const SEARCH_PRS_GRAPHQL_QUERY: &str = r#"
query($q: String!, $after: String) {
  search(type: ISSUE, query: $q, first: 100, after: $after) {
    pageInfo {
      hasNextPage
      endCursor
    }
    nodes {
      ... on PullRequest {
        number
        title
        url
        updatedAt
        reviewDecision
        repository {
          nameWithOwner
        }
        author {
          login
        }
      }
    }
  }
}
"#;

#[derive(Deserialize)]
struct SearchPRsResponse {
    data: SearchData,
}

#[derive(Deserialize)]
struct SearchData {
    search: Search,
}

#[derive(Deserialize)]
struct Search {
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
    nodes: Vec<SearchNode>,
}

#[derive(Deserialize)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct SearchNode {
    number: u64,
    title: String,
    url: String,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
    #[serde(rename = "reviewDecision")]
    review_decision: Option<String>,
    repository: Option<SearchRepo>,
    author: Option<SearchAuthor>,
}

#[derive(Deserialize)]
struct SearchRepo {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: Option<String>,
}

#[derive(Deserialize)]
struct SearchAuthor {
    login: Option<String>,
}

pub struct SearchPRsPage {
    pub prs: Vec<PullRequest>,
    pub has_next_page: bool,
    pub end_cursor: String,
}

pub fn parse_search_prs_response(out: &[u8], label: &str) -> Result<SearchPRsPage> {
    let res: SearchPRsResponse = serde_json::from_slice(out)
        .map_err(|e| crate::error::GhiError::Other(format!("parse search response: {e}")))?;
    let nodes = res.data.search.nodes;
    let mut prs = Vec::with_capacity(nodes.len());
    for item in nodes {
        prs.push(PullRequest {
            repository: item
                .repository
                .and_then(|r| r.name_with_owner)
                .unwrap_or_default(),
            number: item.number,
            title: item.title,
            url: item.url,
            author: item.author.and_then(|a| a.login).unwrap_or_default(),
            updated_at: parse_gh_time(item.updated_at),
            request: label.to_string(),
            review_decision: item.review_decision.unwrap_or_default(),
        });
    }
    Ok(SearchPRsPage {
        prs,
        has_next_page: res.data.search.page_info.has_next_page,
        end_cursor: res.data.search.page_info.end_cursor.unwrap_or_default(),
    })
}

fn parse_gh_time(s: Option<String>) -> SystemTime {
    match s {
        Some(s) => parse_rfc3339(&s),
        None => SystemTime::UNIX_EPOCH,
    }
}

pub fn parse_rfc3339(s: &str) -> SystemTime {
    parse_rfc3339_inner(s).unwrap_or(SystemTime::UNIX_EPOCH)
}

fn parse_rfc3339_inner(s: &str) -> Option<SystemTime> {
    let bytes = s.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        if part.chars().all(|c| c.is_ascii_digit()) {
            part.parse::<i64>().ok()
        } else {
            None
        }
    };
    let year = digits(0..4)?;
    let month = digits(5..7)?;
    let day = digits(8..10)?;
    let hour = digits(11..13)?;
    let minute = digits(14..16)?;
    let second = digits(17..19)?;

    let mut rest = &s[19..];
    let mut nanos = 0i64;
    if rest.starts_with('.') {
        let end = rest[1..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        if end > 1 {
            let frac = &rest[1..end];
            let mut val = frac.to_string();
            while val.len() < 9 {
                val.push('0');
            }
            val.truncate(9);
            nanos = val.parse::<i64>().unwrap_or(0);
        }
        rest = &rest[end..];
    }

    let offset_secs: i64 = if rest == "Z" || rest.is_empty() {
        0
    } else if (rest.starts_with('+') || rest.starts_with('-')) && rest.len() >= 6 {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let hh: i64 = rest.get(1..3)?.parse().ok()?;
        let mm: i64 = rest.get(4..6)?.parse().ok()?;
        sign * (hh * 3600 + mm * 60)
    } else {
        return None;
    };

    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;

    let secs = days * 86400 + hour * 3600 + minute * 60 + second - offset_secs;
    if secs >= 0 {
        return SystemTime::UNIX_EPOCH.checked_add(Duration::new(secs as u64, nanos.max(0) as u32));
    }
    let abs = (-secs) as u64;
    SystemTime::UNIX_EPOCH.checked_sub(Duration::from_secs(abs))
}

pub fn format_human(t: SystemTime) -> String {
    let secs = match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    };
    let (year, month, day, hour, minute, _) = civil_from_unix(secs);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

pub fn format_rfc3339_nanos(t: SystemTime) -> String {
    let (secs, nanos) = match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_nanos()),
        Err(e) => {
            let d = e.duration();
            if d.subsec_nanos() == 0 {
                (-(d.as_secs() as i64), 0)
            } else {
                (-(d.as_secs() as i64) - 1, 1_000_000_000 - d.subsec_nanos())
            }
        }
    };
    let (year, month, day, hour, minute, second) = civil_from_unix(secs);
    let mut frac = String::new();
    if nanos > 0 {
        let s = format!("{nanos:09}");
        let trimmed = s.trim_end_matches('0');
        if !trimmed.is_empty() {
            frac = format!(".{trimmed}");
        }
    }
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{frac}Z")
}

pub fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let hour = (rem / 3600) as u32;
    let minute = ((rem % 3600) / 60) as u32;
    let second = (rem % 60) as u32;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d as u32, hour, minute, second)
}

#[derive(Deserialize)]
#[serde(default)]
struct PRViewResponse {
    number: u64,
    title: String,
    url: String,
    body: String,
    #[serde(rename = "createdAt", deserialize_with = "de_opt_time")]
    created_at: SystemTime,
    #[serde(rename = "updatedAt", deserialize_with = "de_opt_time")]
    updated_at: SystemTime,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "reviewDecision")]
    review_decision: String,
    #[serde(rename = "mergeStateStatus")]
    merge_state_status: String,
    additions: i64,
    deletions: i64,
    #[serde(rename = "changedFiles")]
    changed_files: i64,
    author: Option<SearchAuthor>,
    labels: Vec<LabelNode>,
    #[serde(rename = "latestReviews")]
    latest_reviews: Vec<ReviewNode>,
}

impl Default for PRViewResponse {
    fn default() -> Self {
        PRViewResponse {
            number: 0,
            title: String::new(),
            url: String::new(),
            body: String::new(),
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
            base_ref_name: String::new(),
            head_ref_name: String::new(),
            review_decision: String::new(),
            merge_state_status: String::new(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            author: None,
            labels: Vec::new(),
            latest_reviews: Vec::new(),
        }
    }
}

fn de_opt_time<'de, D>(deserializer: D) -> std::result::Result<SystemTime, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    Ok(parse_gh_time(s))
}

#[derive(Deserialize)]
struct LabelNode {
    name: String,
}

#[derive(Deserialize)]
struct ReviewNode {
    state: Option<String>,
    author: Option<SearchAuthor>,
}

pub async fn load_review_requests(g: &Gh) -> Result<Vec<PullRequest>> {
    let mut queries: Vec<(String, String)> = vec![(
        "@me".to_string(),
        "is:pr is:open archived:false review-requested:@me".to_string(),
    )];

    if let Ok(teams) = load_teams(g).await {
        for t in teams {
            if t.organization.is_empty() || t.slug.is_empty() {
                continue;
            }
            let name = format!("{}/{}", t.organization, t.slug);
            queries.push((
                name.clone(),
                format!("is:pr is:open archived:false team-review-requested:{name}"),
            ));
        }
    }

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    let mut handles = Vec::new();
    for (label, q) in queries {
        let sem = sem.clone();
        let g = g.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;
            let res = search_prs(&g, &q, &label).await;
            (label, res)
        }));
    }

    let mut by_url: Vec<(String, PullRequest)> = Vec::new();
    let mut errs: Vec<String> = Vec::new();
    for handle in handles {
        if let Ok((label, res)) = handle.await {
            match res {
                Ok(prs) => {
                    for pr in prs {
                        if let Some(existing) = by_url.iter_mut().find(|(_, e)| e.url == pr.url) {
                            if !existing.1.request.contains(&pr.request) {
                                existing.1.request =
                                    format!("{}, {}", existing.1.request, pr.request);
                            }
                        } else {
                            by_url.push((pr.url.clone(), pr));
                        }
                    }
                }
                Err(e) => errs.push(format!("{label}: {e}")),
            }
        }
    }

    if by_url.is_empty() && !errs.is_empty() {
        return Err(crate::error::GhiError::Other(errs.join("; ")));
    }

    let mut prs: Vec<PullRequest> = by_url.into_iter().map(|(_, pr)| pr).collect();
    prs.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
    Ok(prs)
}

const TEAMS_CACHE_TTL: Duration = Duration::from_secs(600);

pub async fn load_teams(g: &Gh) -> Result<Vec<Team>> {
    {
        let guard = g.teams_cache.lock().await;
        if let Some((teams, at)) = guard.as_ref() {
            if at.elapsed() < TEAMS_CACHE_TTL {
                return Ok(teams.clone());
            }
        }
    }
    let out = g
        .run(&[
            "api".to_string(),
            "user/teams".to_string(),
            "--paginate".to_string(),
            "--jq".to_string(),
            ".[] | [.organization.login, .slug] | @tsv".to_string(),
        ])
        .await?;
    let text = String::from_utf8_lossy(&out).trim().to_string();
    let teams: Vec<Team> = text
        .split('\n')
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let (org, slug) = line.split_once('\t')?;
            Some(Team {
                organization: org.to_string(),
                slug: slug.to_string(),
            })
        })
        .collect();
    {
        let mut guard = g.teams_cache.lock().await;
        *guard = Some((teams.clone(), Instant::now()));
    }
    Ok(teams)
}

pub async fn search_prs(g: &Gh, query: &str, label: &str) -> Result<Vec<PullRequest>> {
    let mut prs: Vec<PullRequest> = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut args = vec![
            "api".to_string(),
            "graphql".to_string(),
            "-f".to_string(),
            format!("query={SEARCH_PRS_GRAPHQL_QUERY}"),
            "-F".to_string(),
            format!("q={query}"),
        ];
        if let Some(a) = &after {
            args.push("-F".to_string());
            args.push(format!("after={a}"));
        }
        let out = g.run(&args).await?;
        let page = parse_search_prs_response(&out, label)?;
        prs.extend(page.prs);
        if !page.has_next_page || page.end_cursor.is_empty() {
            break;
        }
        after = Some(page.end_cursor);
    }
    Ok(prs)
}

fn detail_json_fields() -> String {
    "number,title,url,body,createdAt,updatedAt,baseRefName,headRefName,reviewDecision,mergeStateStatus,additions,deletions,changedFiles,author,labels,latestReviews"
        .to_string()
}

pub async fn load_pr_detail(g: &Gh, pr: &PullRequest) -> Result<PullRequestDetail> {
    let args = vec![
        "pr".to_string(),
        "view".to_string(),
        pr.url.clone(),
        "--json".to_string(),
        detail_json_fields(),
    ];
    let out = g.run(&args).await?;
    let res: PRViewResponse = serde_json::from_slice(&out)
        .map_err(|e| crate::error::GhiError::Other(format!("parse pr view: {e}")))?;

    let mut detail = PullRequestDetail {
        base: pr.clone(),
        body: res.body,
        created_at: res.created_at,
        base_ref_name: res.base_ref_name,
        head_ref_name: res.head_ref_name,
        merge_state_status: res.merge_state_status,
        additions: res.additions,
        deletions: res.deletions,
        changed_files: res.changed_files,
        ..Default::default()
    };
    if res.number != 0 {
        detail.base.number = res.number;
    }
    if !res.title.is_empty() {
        detail.base.title = res.title.clone();
    }
    if !res.url.is_empty() {
        detail.base.url = res.url.clone();
    }
    if let Some(a) = &res.author {
        let login = a.login.clone().unwrap_or_default();
        if !login.is_empty() {
            detail.base.author = login;
        }
    }
    if res.updated_at != SystemTime::UNIX_EPOCH {
        detail.base.updated_at = res.updated_at;
    }
    detail.base.review_decision = res.review_decision;
    for label in res.labels {
        if !label.name.is_empty() {
            detail.labels.push(label.name);
        }
    }
    for review in res.latest_reviews {
        let login = review.author.and_then(|a| a.login).unwrap_or_default();
        let state = review.state.unwrap_or_default();
        if login.is_empty() || state.is_empty() || state == "PENDING" {
            continue;
        }
        detail.reviewers.push(ReviewSummary {
            author: login,
            state,
        });
    }
    Ok(detail)
}

pub async fn load_diff(g: &Gh, pr: &PullRequest) -> Result<String> {
    let args = vec![
        "pr".to_string(),
        "diff".to_string(),
        pr.url.clone(),
        "--color=always".to_string(),
    ];
    let out = g.run(&args).await?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

pub fn is_pr_diff_too_large_error(err: &crate::error::GhiError) -> bool {
    match err {
        crate::error::GhiError::Gh(m) => {
            m.contains("PullRequest.diff too_large")
                || m.contains("diff exceeded the maximum number of files")
        }
        _ => false,
    }
}

pub async fn approve_pr(g: &Gh, pr: &PullRequest) -> Result<()> {
    let args = vec![
        "pr".to_string(),
        "review".to_string(),
        pr.url.clone(),
        "--approve".to_string(),
    ];
    g.run(&args).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::GhiError;

    #[test]
    fn test_is_pr_diff_too_large_error() {
        let cases: Vec<(GhiError, bool)> = vec![
            (
                GhiError::Gh(
                    "gh pr diff https://github.com/example/repo/pull/1 --color=always: could not find pull request diff: HTTP 406: Sorry, the diff exceeded the maximum number of files (300). PullRequest.diff too_large".into(),
                ),
                true,
            ),
            (
                GhiError::Gh("HTTP 406: Sorry, the diff exceeded the maximum number of files (300). Consider using 'List pull requests files' API".into()),
                true,
            ),
            (GhiError::Gh("gh pr diff: HTTP 404: Not Found".into()), false),
        ];

        for (err, want) in cases {
            assert_eq!(is_pr_diff_too_large_error(&err), want);
        }
    }

    #[test]
    fn test_parse_search_prs_response_includes_review_decision() {
        let out = br#"{
            "data": {
                "search": {
                    "pageInfo": {"hasNextPage": true, "endCursor": "cursor-1"},
                    "nodes": [
                        {
                            "number": 42,
                            "title": "Add review list status",
                            "url": "https://github.com/owner/repo/pull/42",
                            "updatedAt": "2026-05-13T01:02:03Z",
                            "reviewDecision": "APPROVED",
                            "repository": {"nameWithOwner": "owner/repo"},
                            "author": {"login": "octocat"}
                        }
                    ]
                }
            }
        }"#;

        let page = parse_search_prs_response(out, "@me").unwrap();
        assert!(page.has_next_page);
        assert_eq!(page.end_cursor, "cursor-1");
        assert_eq!(page.prs.len(), 1);

        let pr = &page.prs[0];
        assert_eq!(pr.repository, "owner/repo");
        assert_eq!(pr.review_decision, "APPROVED");
        let want = SystemTime::UNIX_EPOCH + Duration::from_secs(1778634123);
        assert_eq!(pr.updated_at, want);
    }

    #[test]
    fn test_parse_rfc3339_with_offset() {
        let got = parse_rfc3339("2026-05-13T09:02:03+08:00");
        let want = SystemTime::UNIX_EPOCH + Duration::from_secs(1778634123);
        assert_eq!(got, want);
    }

    #[test]
    fn test_format_rfc3339_nanos_roundtrip() {
        let t = SystemTime::UNIX_EPOCH + Duration::new(1778634123, 123_000_000);
        let s = format_rfc3339_nanos(t);
        assert_eq!(s, "2026-05-13T01:02:03.123Z");
        assert_eq!(parse_rfc3339(&s), t);
    }
}
