//! Lane inference for the agents sidebar.
//!
//! Kept free of I/O and of Ratatui so tests can pin every transition.

use std::fmt;

/// Display lane shown in the sidebar. New variants can be added without
/// changing existing match arms that use `_` for unknown future lanes in
/// call sites that only care about the MVP set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AgentLane {
    NeedsAttention,
    Waiting,
    Working,
    Idle,
    Done,
}

impl AgentLane {
    pub const ALL: [AgentLane; 5] = [
        AgentLane::NeedsAttention,
        AgentLane::Working,
        AgentLane::Waiting,
        AgentLane::Done,
        AgentLane::Idle,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::NeedsAttention => "NEEDS ATTENTION",
            Self::Working => "WORKING",
            Self::Waiting => "WAITING",
            Self::Done => "DONE",
            Self::Idle => "IDLE",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::NeedsAttention => "⚠",
            Self::Working => "●",
            Self::Waiting => "◐",
            Self::Done => "✓",
            Self::Idle => "○",
        }
    }
}

/// Public cmux agent projection state. Names match `cmux.protocol/2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmuxAgentState {
    Working,
    Blocked,
    Idle,
    Done,
    Unknown,
}

impl CmuxAgentState {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "working" => Self::Working,
            "blocked" => Self::Blocked,
            "idle" => Self::Idle,
            "done" => Self::Done,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Idle => "idle",
            Self::Done => "done",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CiStatus {
    #[default]
    Unknown,
    Pending,
    Passing,
    Failing,
}

/// GitHub `reviewDecision`. Empty / missing is Unknown, not ReviewRequired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewDecision {
    #[default]
    Unknown,
    Approved,
    ReviewRequired,
    ChangesRequested,
}

impl ReviewDecision {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_uppercase().as_str() {
            "APPROVED" => Self::Approved,
            "REVIEW_REQUIRED" => Self::ReviewRequired,
            "CHANGES_REQUESTED" => Self::ChangesRequested,
            _ => Self::Unknown,
        }
    }

    pub fn is_requested(self) -> bool {
        matches!(self, Self::ReviewRequired | Self::ChangesRequested)
    }
}

/// GitHub GraphQL `mergeStateStatus` (Merge button), not `mergeable` (conflicts only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergeStateStatus {
    #[default]
    Unknown,
    Clean,
    Blocked,
    Behind,
    Dirty,
    Draft,
    Unstable,
}

impl MergeStateStatus {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_uppercase().as_str() {
            "CLEAN" | "HAS_HOOKS" => Self::Clean,
            "BLOCKED" => Self::Blocked,
            "BEHIND" => Self::Behind,
            "DIRTY" => Self::Dirty,
            "DRAFT" => Self::Draft,
            "UNSTABLE" => Self::Unstable,
            _ => Self::Unknown,
        }
    }

    pub fn from_gh(merge_state_status: &str, mergeable: &str) -> Self {
        let parsed = Self::parse(merge_state_status);
        if parsed != Self::Unknown {
            return parsed;
        }
        match mergeable.trim().to_ascii_uppercase().as_str() {
            "CONFLICTING" => Self::Dirty,
            _ => Self::Unknown,
        }
    }

    pub fn blocker_label(self) -> Option<&'static str> {
        match self {
            Self::Dirty => Some("Conflict"),
            Self::Behind => Some("Behind"),
            Self::Blocked => Some("Blocked"),
            Self::Unstable => Some("Checks failing"),
            Self::Draft => Some("Draft"),
            Self::Clean | Self::Unknown => None,
        }
    }
}

impl CiStatus {
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Unknown => None,
            Self::Pending => Some("CI pending"),
            Self::Passing => Some("✓ CI"),
            Self::Failing => Some("CI failing"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitHubSignals {
    pub available: bool,
    pub pr_number: Option<u32>,
    pub pr_open: bool,
    pub pr_merged: bool,
    pub pr_draft: bool,
    pub review_decision: ReviewDecision,
    pub review_requested: bool,
    pub merge_state: MergeStateStatus,
    pub ci: CiStatus,
    pub branch: Option<String>,
}

impl GitHubSignals {
    pub fn unavailable() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceSignals {
    pub agent: Option<CmuxAgentState>,
    pub unread_notification: bool,
    pub notification_hint: Option<String>,
    pub github: GitHubSignals,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedState {
    pub lane: AgentLane,
    pub detail: String,
}

impl fmt::Display for ResolvedState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} — {}", self.lane.title(), self.detail)
    }
}

/// Pure classifier. UI must not duplicate these rules.
pub struct StateResolver;

impl StateResolver {
    pub fn resolve(signals: &WorkspaceSignals) -> ResolvedState {
        if needs_attention(signals) {
            return ResolvedState {
                lane: AgentLane::NeedsAttention,
                detail: attention_detail(signals),
            };
        }

        let agent_busy = matches!(signals.agent, Some(CmuxAgentState::Working));

        // Open PR + idle agent = waiting on reviewers / merge, not on us.
        if signals.github.available && signals.github.pr_open && !agent_busy {
            return ResolvedState {
                lane: AgentLane::Waiting,
                detail: review_detail(signals),
            };
        }

        if agent_busy {
            return ResolvedState {
                lane: AgentLane::Working,
                detail: working_detail(signals),
            };
        }

        if signals.github.available && signals.github.pr_merged && !signals.github.pr_open {
            return ResolvedState {
                lane: AgentLane::Done,
                detail: "Merged".to_string(),
            };
        }

        if matches!(signals.agent, Some(CmuxAgentState::Done)) {
            return ResolvedState {
                lane: AgentLane::Done,
                detail: "Done".to_string(),
            };
        }

        ResolvedState {
            lane: AgentLane::Idle,
            detail: idle_detail(signals),
        }
    }
}

fn needs_attention(signals: &WorkspaceSignals) -> bool {
    signals.unread_notification || matches!(signals.agent, Some(CmuxAgentState::Blocked))
}

fn attention_detail(signals: &WorkspaceSignals) -> String {
    if let Some(hint) = signals
        .notification_hint
        .as_deref()
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
    {
        return hint.to_string();
    }
    if matches!(signals.agent, Some(CmuxAgentState::Blocked)) {
        "Waiting for input".to_string()
    } else {
        "Needs attention".to_string()
    }
}

fn review_detail(signals: &WorkspaceSignals) -> String {
    let mut parts = Vec::new();
    if let Some(ci) = signals.github.ci.label() {
        parts.push(ci.to_string());
    }
    if signals.github.pr_draft {
        parts.push("Draft".to_string());
    } else if signals.github.review_decision == ReviewDecision::Approved {
        parts.push("Approved".to_string());
        if signals.github.merge_state == MergeStateStatus::Clean {
            parts.push("Ready to merge".to_string());
        } else if let Some(blocker) = signals.github.merge_state.blocker_label() {
            parts.push(blocker.to_string());
        }
    } else if signals.github.review_requested {
        parts.push("Review pending".to_string());
    } else {
        parts.push("PR open".to_string());
    }
    parts.join(" · ")
}

fn working_detail(signals: &WorkspaceSignals) -> String {
    signals
        .github
        .branch
        .clone()
        .unwrap_or_else(|| "Working".to_string())
}

fn idle_detail(signals: &WorkspaceSignals) -> String {
    match signals.agent {
        Some(CmuxAgentState::Idle) => "Idle".to_string(),
        Some(CmuxAgentState::Unknown) => "Agent unknown".to_string(),
        None => "Idle".to_string(),
        _ => "Idle".to_string(),
    }
}
