use cmux_agents_sidebar::state::AgentLane;
use cmux_agents_sidebar::state::{
    CiStatus, CmuxAgentState, GitHubSignals, MergeStateStatus, ReviewDecision, StateResolver,
    WorkspaceSignals,
};

fn github_open_review() -> GitHubSignals {
    GitHubSignals {
        available: true,
        pr_number: Some(184),
        pr_open: true,
        pr_merged: false,
        pr_draft: false,
        review_decision: ReviewDecision::ReviewRequired,
        review_requested: true,
        merge_state: MergeStateStatus::Unknown,
        ci: CiStatus::Passing,
        branch: Some("feature/auth".into()),
    }
}

fn github_approved(merge_state: MergeStateStatus) -> GitHubSignals {
    GitHubSignals {
        available: true,
        pr_number: Some(200),
        pr_open: true,
        pr_merged: false,
        pr_draft: false,
        review_decision: ReviewDecision::Approved,
        review_requested: false,
        merge_state,
        ci: CiStatus::Passing,
        branch: Some("feature/ready".into()),
    }
}

#[test]
fn agent_running_is_working() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Working),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::Working);
}

#[test]
fn agent_running_plus_unread_notification_is_needs_attention() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Working),
        unread_notification: true,
        notification_hint: Some("Claude finished".into()),
        github: GitHubSignals::unavailable(),
    });
    assert_eq!(resolved.lane, AgentLane::NeedsAttention);
    assert_eq!(resolved.detail, "Claude finished");
}

#[test]
fn blocked_agent_is_needs_attention() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Blocked),
        unread_notification: false,
        github: GitHubSignals::unavailable(),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::NeedsAttention);
    assert_eq!(resolved.detail, "Waiting for input");
}

#[test]
fn open_pr_with_review_requested_is_in_review() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Idle),
        github: github_open_review(),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::InReview);
    assert!(resolved.detail.contains("Review pending"));
    assert!(resolved.detail.contains("✓ CI"));
}

#[test]
fn working_agent_beats_open_pr() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Working),
        github: github_open_review(),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::Working);
}

#[test]
fn approved_clean_pr_is_ready_to_merge() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Idle),
        github: github_approved(MergeStateStatus::Clean),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::InReview);
    assert!(resolved.detail.contains("Approved"));
    assert!(resolved.detail.contains("Ready to merge"));
}

#[test]
fn approved_conflicting_pr_shows_conflict() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Idle),
        github: github_approved(MergeStateStatus::Dirty),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::InReview);
    assert!(resolved.detail.contains("Approved"));
    assert!(resolved.detail.contains("Conflict"));
    assert!(!resolved.detail.contains("Ready to merge"));
}

#[test]
fn merged_pr_is_done() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Idle),
        github: GitHubSignals {
            available: true,
            pr_number: Some(179),
            pr_open: false,
            pr_merged: true,
            ..GitHubSignals::default()
        },
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::Done);
    assert_eq!(resolved.detail, "Merged");
}

#[test]
fn github_unavailable_with_running_agent_is_working() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Working),
        github: GitHubSignals::unavailable(),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::Working);
}

#[test]
fn unknown_agent_still_classifies_idle() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Unknown),
        github: GitHubSignals::unavailable(),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::Idle);
}

#[test]
fn unknown_agent_with_unread_notification_is_attention() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Unknown),
        unread_notification: true,
        github: GitHubSignals::unavailable(),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::NeedsAttention);
}

#[test]
fn agent_done_without_pr_is_done() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Done),
        github: GitHubSignals::unavailable(),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::Done);
}

#[test]
fn attention_beats_review_and_working() {
    let resolved = StateResolver::resolve(&WorkspaceSignals {
        agent: Some(CmuxAgentState::Working),
        unread_notification: true,
        github: github_open_review(),
        ..WorkspaceSignals::default()
    });
    assert_eq!(resolved.lane, AgentLane::NeedsAttention);
}

#[test]
fn no_agent_no_github_is_idle() {
    let resolved = StateResolver::resolve(&WorkspaceSignals::default());
    assert_eq!(resolved.lane, AgentLane::Idle);
}
