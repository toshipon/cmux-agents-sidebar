//! Optional GitHub signals via `gh`. Never required for the sidebar to run.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::state::{CiStatus, GitHubSignals, MergeStateStatus, ReviewDecision, ReviewTurn};

const GH_TTL: Duration = Duration::from_secs(20);
const GH_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone)]
struct CacheEntry {
    fetched_at: Instant,
    signals: GitHubSignals,
}

#[derive(Debug, Default)]
pub struct GitHubProbe {
    gh_available: Option<bool>,
    cache: Vec<(PathBuf, CacheEntry)>,
    fetches_this_tick: u8,
}

impl GitHubProbe {
    pub fn begin_tick(&mut self) {
        self.fetches_this_tick = 0;
    }

    pub fn signals_for_cwd(&mut self, cwd: Option<&Path>) -> GitHubSignals {
        let Some(cwd) = cwd else {
            return GitHubSignals::unavailable();
        };
        if !cwd.is_dir() {
            return GitHubSignals::unavailable();
        }
        if let Some(entry) = self.cache.iter().find(|(path, _)| path == cwd)
            && entry.1.fetched_at.elapsed() < GH_TTL
        {
            return entry.1.signals.clone();
        }
        if self.fetches_this_tick >= 2 {
            if let Some(entry) = self.cache.iter().find(|(path, _)| path == cwd) {
                return entry.1.signals.clone();
            }
            return GitHubSignals::unavailable();
        }
        self.fetches_this_tick = self.fetches_this_tick.saturating_add(1);
        let signals = self.fetch(cwd);
        self.cache.retain(|(path, _)| path != cwd);
        self.cache.push((
            cwd.to_path_buf(),
            CacheEntry {
                fetched_at: Instant::now(),
                signals: signals.clone(),
            },
        ));
        if self.cache.len() > 64 {
            self.cache.remove(0);
        }
        signals
    }

    fn gh_on_path(&mut self) -> bool {
        if let Some(known) = self.gh_available {
            return known;
        }
        let ok = Command::new("gh")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        self.gh_available = Some(ok);
        ok
    }

    fn fetch(&mut self, cwd: &Path) -> GitHubSignals {
        if !self.gh_on_path() {
            return GitHubSignals::unavailable();
        }
        if !is_git_repo(cwd) {
            return GitHubSignals::unavailable();
        }
        let branch = git_branch(cwd);
        match gh_pr_view(cwd) {
            Some(value) => {
                let mut signals = parse_pr_view(&value, branch);
                if signals.pr_open {
                    signals.review_turn = review_turn_for_pr(cwd, &value, signals.pr_number);
                }
                signals
            }
            None => GitHubSignals {
                available: true,
                branch,
                ..GitHubSignals::unavailable()
            },
        }
    }
}

fn is_git_repo(cwd: &Path) -> bool {
    Command::new("git")
        .args([
            "-C",
            cwd.to_str().unwrap_or("."),
            "rev-parse",
            "--is-inside-work-tree",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn git_branch(cwd: &Path) -> Option<String> {
    let output = Command::new("git")
        .args([
            "-C",
            cwd.to_str().unwrap_or("."),
            "branch",
            "--show-current",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}

fn gh_pr_view(cwd: &Path) -> Option<Value> {
    let mut command = Command::new("gh");
    command
        .current_dir(cwd)
        .args([
            "pr",
            "view",
            "--json",
            "number,state,isDraft,author,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup,mergedAt,headRefName,url,title",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let output = spawn_with_timeout(command)?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn spawn_with_timeout(mut command: Command) -> Option<std::process::Output> {
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    let timeout = Duration::from_secs(GH_TIMEOUT_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_end(&mut stdout);
                }
                return Some(std::process::Output {
                    status,
                    stdout,
                    stderr: Vec::new(),
                });
            }
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

pub fn parse_pr_view(value: &Value, branch: Option<String>) -> GitHubSignals {
    let number = value
        .get("number")
        .and_then(Value::as_u64)
        .map(|n| n as u32);
    let state = value.get("state").and_then(Value::as_str).unwrap_or("");
    let merged = value.get("mergedAt").and_then(Value::as_str).is_some()
        || state.eq_ignore_ascii_case("merged");
    let open = state.eq_ignore_ascii_case("open");
    let draft = value
        .get("isDraft")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let review = value
        .get("reviewDecision")
        .and_then(Value::as_str)
        .unwrap_or("");
    let review_decision = ReviewDecision::parse(review);
    let review_requested = review_decision.is_requested();
    let merge_state = MergeStateStatus::from_gh(
        value
            .get("mergeStateStatus")
            .and_then(Value::as_str)
            .unwrap_or(""),
        value.get("mergeable").and_then(Value::as_str).unwrap_or(""),
    );
    let branch = value
        .get("headRefName")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(branch);
    GitHubSignals {
        available: true,
        pr_number: number,
        pr_open: open,
        pr_merged: merged,
        pr_draft: draft,
        review_decision,
        review_requested,
        merge_state,
        review_turn: ReviewTurn::Unknown,
        ci: ci_from_rollup(value.get("statusCheckRollup")),
        branch,
    }
}

const REVIEW_THREADS_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!){viewer{login}repository(owner:$owner,name:$name){pullRequest(number:$number){reviewThreads(first:50){nodes{isResolved comments(last:1){nodes{author{login} authorAssociation}}}}}}}";

fn review_turn_for_pr(cwd: &Path, pr_view: &Value, number: Option<u32>) -> ReviewTurn {
    let url = pr_view.get("url").and_then(Value::as_str).unwrap_or("");
    let Some((owner, name, url_number)) = parse_github_pr_url(url) else {
        return ReviewTurn::Unknown;
    };
    let number = number.unwrap_or(url_number);
    let Some(payload) = gh_review_threads(cwd, &owner, &name, number) else {
        return ReviewTurn::Unknown;
    };
    let author = pr_author_login(pr_view).unwrap_or_default();
    parse_review_turn(&payload, &author)
}

fn pr_author_login(value: &Value) -> Option<String> {
    let author = value.get("author")?;
    if let Some(login) = author.as_str() {
        return Some(login.to_string());
    }
    author
        .get("login")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub fn parse_github_pr_url(url: &str) -> Option<(String, String, u32)> {
    let trimmed = url.trim();
    let path = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let _host = parts.next()?;
    let owner = parts.next()?.to_string();
    let name = parts.next()?.to_string();
    if parts.next()? != "pull" {
        return None;
    }
    let number = parts
        .next()?
        .split(['#', '?'])
        .next()?
        .parse::<u32>()
        .ok()?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some((owner, name, number))
}

fn gh_review_threads(cwd: &Path, owner: &str, name: &str, number: u32) -> Option<Value> {
    let owner_field = format!("owner={owner}");
    let name_field = format!("name={name}");
    let number_field = format!("number={number}");
    let query_field = format!("query={REVIEW_THREADS_QUERY}");
    let mut command = Command::new("gh");
    command
        .current_dir(cwd)
        .args([
            "api",
            "graphql",
            "-f",
            &owner_field,
            "-f",
            &name_field,
            "-F",
            &number_field,
            "-f",
            &query_field,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let output = spawn_with_timeout(command)?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

pub fn parse_review_turn(payload: &Value, pr_author: &str) -> ReviewTurn {
    let data = payload.get("data").unwrap_or(payload);
    let viewer = data
        .get("viewer")
        .and_then(|viewer| viewer.get("login"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if viewer.is_empty() || pr_author.is_empty() || viewer.eq_ignore_ascii_case(pr_author) {
        return ReviewTurn::Unknown;
    }
    let nodes = data
        .get("repository")
        .and_then(|repo| repo.get("pullRequest"))
        .and_then(|pr| pr.get("reviewThreads"))
        .and_then(|threads| threads.get("nodes"))
        .and_then(Value::as_array);
    let Some(nodes) = nodes else {
        return ReviewTurn::Unknown;
    };
    let mut awaiting = false;
    let mut needs_reply = false;
    for thread in nodes {
        if thread
            .get("isResolved")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let last = thread
            .get("comments")
            .and_then(|comments| comments.get("nodes"))
            .and_then(Value::as_array)
            .and_then(|comments| comments.last());
        let Some(last) = last else {
            continue;
        };
        let login = last
            .get("author")
            .and_then(|author| author.get("login"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let association = last
            .get("authorAssociation")
            .and_then(Value::as_str)
            .unwrap_or("");
        if login.is_empty() || is_bot_comment(login, association) {
            continue;
        }
        if login.eq_ignore_ascii_case(viewer) {
            awaiting = true;
        } else {
            needs_reply = true;
        }
    }
    if needs_reply {
        ReviewTurn::NeedsReply
    } else if awaiting {
        ReviewTurn::AwaitingReply
    } else {
        ReviewTurn::Unknown
    }
}

fn is_bot_comment(login: &str, association: &str) -> bool {
    if association.eq_ignore_ascii_case("BOT") {
        return true;
    }
    let lower = login.to_ascii_lowercase();
    lower.ends_with("[bot]")
        || lower.ends_with("-bot")
        || lower == "copilot"
        || lower == "dependabot"
        || lower == "renovate"
        || lower == "github-actions"
        || lower == "github-actions[bot]"
}

fn ci_from_rollup(value: Option<&Value>) -> CiStatus {
    let Some(Value::Array(checks)) = value else {
        return CiStatus::Unknown;
    };
    if checks.is_empty() {
        return CiStatus::Unknown;
    }
    let mut pending = false;
    let mut failing = false;
    for check in checks {
        let conclusion = check
            .get("conclusion")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_uppercase();
        let status = check
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_uppercase();
        if is_failing_check(&conclusion) {
            failing = true;
        } else if is_pending_check(&conclusion, &status) {
            pending = true;
        }
    }
    if failing {
        CiStatus::Failing
    } else if pending {
        CiStatus::Pending
    } else {
        CiStatus::Passing
    }
}

fn is_failing_check(conclusion: &str) -> bool {
    matches!(
        conclusion,
        "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
    )
}

fn is_pending_check(conclusion: &str, status: &str) -> bool {
    if matches!(conclusion, "SUCCESS" | "SKIPPED") {
        return false;
    }
    conclusion.is_empty()
        || conclusion == "NEUTRAL"
        || matches!(status, "IN_PROGRESS" | "QUEUED" | "PENDING" | "EXPECTED")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{MergeStateStatus, ReviewDecision, ReviewTurn};

    #[test]
    fn parses_open_review_requested_pr() {
        let json = serde_json::json!({
            "number": 184,
            "state": "OPEN",
            "isDraft": false,
            "reviewDecision": "REVIEW_REQUIRED",
            "mergedAt": null,
            "headRefName": "feature/auth",
            "statusCheckRollup": [
                {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"}
            ]
        });
        let signals = parse_pr_view(&json, None);
        assert_eq!(signals.pr_number, Some(184));
        assert!(signals.pr_open);
        assert!(!signals.pr_merged);
        assert_eq!(signals.review_decision, ReviewDecision::ReviewRequired);
        assert!(signals.review_requested);
        assert_eq!(signals.ci, CiStatus::Passing);
        assert_eq!(signals.branch.as_deref(), Some("feature/auth"));
    }

    #[test]
    fn parses_approved_ready_to_merge() {
        let json = serde_json::json!({
            "number": 200,
            "state": "OPEN",
            "isDraft": false,
            "reviewDecision": "APPROVED",
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "mergedAt": null,
            "headRefName": "feature/ready",
            "statusCheckRollup": [
                {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"}
            ]
        });
        let signals = parse_pr_view(&json, None);
        assert_eq!(signals.review_decision, ReviewDecision::Approved);
        assert!(!signals.review_requested);
        assert_eq!(signals.merge_state, MergeStateStatus::Clean);
    }

    #[test]
    fn conflicting_mergeable_is_dirty_without_status() {
        let json = serde_json::json!({
            "number": 201,
            "state": "OPEN",
            "reviewDecision": "APPROVED",
            "mergeable": "CONFLICTING",
        });
        let signals = parse_pr_view(&json, None);
        assert_eq!(signals.merge_state, MergeStateStatus::Dirty);
    }

    #[test]
    fn parses_merged_pr() {
        let json = serde_json::json!({
            "number": 179,
            "state": "MERGED",
            "isDraft": false,
            "reviewDecision": "APPROVED",
            "mergedAt": "2026-01-01T00:00:00Z",
            "headRefName": "renovate",
            "statusCheckRollup": []
        });
        let signals = parse_pr_view(&json, None);
        assert!(signals.pr_merged);
        assert!(!signals.pr_open);
        assert_eq!(signals.ci, CiStatus::Unknown);
    }

    #[test]
    fn failing_check_wins() {
        let json = serde_json::json!({
            "number": 1,
            "state": "OPEN",
            "statusCheckRollup": [
                {"conclusion": "SUCCESS", "status": "COMPLETED"},
                {"conclusion": "FAILURE", "status": "COMPLETED"}
            ]
        });
        assert_eq!(parse_pr_view(&json, None).ci, CiStatus::Failing);
    }

    fn thread_payload(viewer: &str, threads: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({
            "data": {
                "viewer": { "login": viewer },
                "repository": {
                    "pullRequest": {
                        "reviewThreads": { "nodes": threads }
                    }
                }
            }
        })
    }

    fn thread(resolved: bool, login: &str, association: &str) -> serde_json::Value {
        serde_json::json!({
            "isResolved": resolved,
            "comments": {
                "nodes": [{
                    "author": { "login": login },
                    "authorAssociation": association
                }]
            }
        })
    }

    #[test]
    fn parses_github_pr_url() {
        let parsed = parse_github_pr_url("https://github.com/acme/app/pull/42#discussion").unwrap();
        assert_eq!(parsed, ("acme".into(), "app".into(), 42));
    }

    #[test]
    fn reviewer_last_comment_is_awaiting_reply() {
        let payload = thread_payload("reviewer", vec![thread(false, "reviewer", "MEMBER")]);
        assert_eq!(
            parse_review_turn(&payload, "author"),
            ReviewTurn::AwaitingReply
        );
    }

    #[test]
    fn other_last_comment_is_needs_reply() {
        let payload = thread_payload("reviewer", vec![thread(false, "author", "OWNER")]);
        assert_eq!(
            parse_review_turn(&payload, "author"),
            ReviewTurn::NeedsReply
        );
    }

    #[test]
    fn mixed_threads_prefer_needs_reply() {
        let payload = thread_payload(
            "reviewer",
            vec![
                thread(false, "reviewer", "MEMBER"),
                thread(false, "author", "OWNER"),
            ],
        );
        assert_eq!(
            parse_review_turn(&payload, "author"),
            ReviewTurn::NeedsReply
        );
    }

    #[test]
    fn resolved_and_bot_threads_are_ignored() {
        let payload = thread_payload(
            "reviewer",
            vec![
                thread(true, "author", "OWNER"),
                thread(false, "copilot[bot]", "BOT"),
                thread(false, "reviewer", "MEMBER"),
            ],
        );
        assert_eq!(
            parse_review_turn(&payload, "author"),
            ReviewTurn::AwaitingReply
        );
    }

    #[test]
    fn author_workspace_does_not_get_a_review_turn() {
        let payload = thread_payload("author", vec![thread(false, "reviewer", "MEMBER")]);
        assert_eq!(parse_review_turn(&payload, "author"), ReviewTurn::Unknown);
    }
}
