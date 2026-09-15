//! Optional GitHub signals via `gh`. Never required for the sidebar to run.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::state::{CiStatus, GitHubSignals};

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
            Some(value) => parse_pr_view(&value, branch),
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
            "number,state,isDraft,reviewDecision,statusCheckRollup,mergedAt,headRefName,url,title",
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
    let review_requested = review.eq_ignore_ascii_case("review_required")
        || review.eq_ignore_ascii_case("changes_requested");
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
        review_requested,
        ci: ci_from_rollup(value.get("statusCheckRollup")),
        branch,
    }
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
        assert!(signals.review_requested);
        assert_eq!(signals.ci, CiStatus::Passing);
        assert_eq!(signals.branch.as_deref(), Some("feature/auth"));
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
}
