//! Workspace-first view model. StateResolver stays I/O-free; this module
//! only joins cmux snapshots with GitHub and detection.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::agent::{AgentDetector, AgentKind, CompositeAgentDetector, DetectInput};
use crate::cmux::{self, NotificationSnapshot, SessionSnapshot};
use crate::github::GitHubProbe;
use crate::state::{
    AgentLane, CmuxAgentState, GitHubSignals, ResolvedState, StateResolver, WorkspaceSignals,
};

#[derive(Debug, Clone)]
pub struct Task {
    pub workspace_id: String,
    pub workspace_name: String,
    pub tab_id: Option<String>,
    pub focused: bool,
    pub kind: AgentKind,
    pub extra_agents: usize,
    pub lane: AgentLane,
    pub detail: String,
    pub subtitle: String,
    pub age_ms: Option<u64>,
    pub pr_number: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct GroupedTasks {
    pub groups: Vec<(AgentLane, Vec<Task>)>,
}

pub fn build_tasks(snapshot: &SessionSnapshot, github: &mut GitHubProbe, now_ms: u64) -> Vec<Task> {
    let detector = CompositeAgentDetector::default();
    let terminals = cmux::index_terminals(snapshot);
    let mut by_workspace: BTreeMap<String, WorkspaceAcc> = BTreeMap::new();

    for workspace in &snapshot.workspaces {
        by_workspace
            .entry(workspace.id.clone())
            .or_insert_with(|| WorkspaceAcc {
                name: workspace.name.clone(),
                focused: workspace.focused,
                ..WorkspaceAcc::default()
            });
        if let Some(acc) = by_workspace.get_mut(&workspace.id) {
            acc.name = workspace.name.clone();
            acc.focused = workspace.focused;
        }
    }

    for agent in &snapshot.agents {
        let term = terminals.get(&agent.terminal_id);
        let workspace_id = term
            .map(|term| term.workspace_id.clone())
            .filter(|id| !id.is_empty());
        let Some(workspace_id) = workspace_id else {
            continue;
        };
        let acc = by_workspace.entry(workspace_id.clone()).or_default();
        let workspace_name = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .map(|workspace| workspace.name.as_str())
            .unwrap_or("");
        let kind = detector.detect(&DetectInput {
            workspace_name,
            tab_name: term.and_then(|term| term.tab_name.as_deref()),
            terminal_title: term.map(|term| term.title.as_str()),
            adapter_id: None,
        });
        acc.agents.push(AgentAcc {
            state: CmuxAgentState::parse(&agent.state),
            kind,
            tab_id: term.and_then(|term| term.tab_id.clone()),
            cwd: term.and_then(|term| term.cwd.clone()),
            updated_at_ms: agent.updated_at_ms,
        });
    }

    for notification in &snapshot.notifications {
        if !notification.unread {
            continue;
        }
        let workspace_id = notification
            .terminal_id
            .as_ref()
            .and_then(|id| terminals.get(id))
            .map(|term| term.workspace_id.clone())
            .filter(|id| !id.is_empty());
        let Some(workspace_id) = workspace_id else {
            continue;
        };
        let acc = by_workspace.entry(workspace_id).or_default();
        acc.unread_notification = true;
        if acc.notification_hint.is_none() {
            acc.notification_hint = notification_hint(notification);
        }
        acc.newest_ms = newest(acc.newest_ms, notification.created_at_ms);
    }

    github.begin_tick();
    let mut tasks = Vec::new();
    for (workspace_id, acc) in by_workspace {
        if acc.name.is_empty() && acc.agents.is_empty() && !acc.unread_notification {
            continue;
        }
        let primary = pick_primary(&acc.agents);
        let cwd = primary.and_then(|agent| agent.cwd.as_deref());
        let github_signals = github.signals_for_cwd(cwd);
        let agent_state = primary
            .map(|agent| agent.state)
            .or_else(|| acc.agents.first().map(|agent| agent.state));
        let signals = WorkspaceSignals {
            agent: agent_state,
            unread_notification: acc.unread_notification,
            notification_hint: acc.notification_hint.clone(),
            github: github_signals.clone(),
        };
        let resolved = StateResolver::resolve(&signals);
        let kind = primary.map(|agent| agent.kind).unwrap_or(AgentKind::Other);
        let extra_agents = acc.agents.len().saturating_sub(1);
        let age_ms = newest(
            newest(acc.newest_ms, primary.and_then(|agent| agent.updated_at_ms)),
            None,
        )
        .and_then(|then| now_ms.checked_sub(then));
        tasks.push(Task {
            workspace_id,
            workspace_name: display_name(&acc.name),
            tab_id: primary.and_then(|agent| agent.tab_id.clone()),
            focused: acc.focused,
            kind,
            extra_agents,
            lane: resolved.lane,
            detail: resolved.detail.clone(),
            subtitle: subtitle(&resolved, &github_signals, kind, extra_agents, age_ms),
            age_ms,
            pr_number: github_signals.pr_number,
        });
    }

    tasks.sort_by(|a, b| {
        lane_rank(a.lane).cmp(&lane_rank(b.lane)).then(
            a.workspace_name
                .to_ascii_lowercase()
                .cmp(&b.workspace_name.to_ascii_lowercase()),
        )
    });
    tasks
}

pub fn group_tasks(tasks: Vec<Task>) -> GroupedTasks {
    let mut groups = Vec::new();
    for lane in AgentLane::ALL {
        let members: Vec<Task> = tasks
            .iter()
            .filter(|task| task.lane == lane)
            .cloned()
            .collect();
        if !members.is_empty() {
            groups.push((lane, members));
        }
    }
    GroupedTasks { groups }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn format_age(age_ms: Option<u64>) -> Option<String> {
    let age_ms = age_ms?;
    let secs = age_ms / 1000;
    if secs < 60 {
        Some(format!("{secs}s"))
    } else if secs < 3600 {
        Some(format!("{}m", secs / 60))
    } else {
        Some(format!("{}h", secs / 3600))
    }
}

#[derive(Default)]
struct WorkspaceAcc {
    name: String,
    focused: bool,
    agents: Vec<AgentAcc>,
    unread_notification: bool,
    notification_hint: Option<String>,
    newest_ms: Option<u64>,
}

#[derive(Clone)]
struct AgentAcc {
    state: CmuxAgentState,
    kind: AgentKind,
    tab_id: Option<String>,
    cwd: Option<PathBuf>,
    updated_at_ms: Option<u64>,
}

fn pick_primary(agents: &[AgentAcc]) -> Option<&AgentAcc> {
    agents.iter().min_by_key(|agent| agent_rank(agent.state))
}

fn agent_rank(state: CmuxAgentState) -> u8 {
    match state {
        CmuxAgentState::Blocked => 0,
        CmuxAgentState::Working => 1,
        CmuxAgentState::Idle => 2,
        CmuxAgentState::Unknown => 3,
        CmuxAgentState::Done => 4,
    }
}

fn lane_rank(lane: AgentLane) -> u8 {
    match lane {
        AgentLane::NeedsAttention => 0,
        AgentLane::Working => 1,
        AgentLane::InReview => 2,
        AgentLane::Done => 3,
        AgentLane::Idle => 4,
    }
}

fn newest(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn notification_hint(notification: &NotificationSnapshot) -> Option<String> {
    let title = notification.title.to_ascii_lowercase();
    if title.contains("approval") || title.contains("permission") {
        Some("Permission required".into())
    } else if title.contains("question") || title.contains("asked") {
        Some("Waiting for input".into())
    } else if title.contains("finished") || title.contains("done") {
        Some("Review completed work".into())
    } else if !notification.title.is_empty() {
        Some(notification.title.clone())
    } else if !notification.body.is_empty() {
        Some(notification.body.clone())
    } else {
        None
    }
}

fn display_name(name: &str) -> String {
    if name.trim().is_empty() {
        "workspace".to_string()
    } else {
        name.to_string()
    }
}

fn subtitle(
    resolved: &ResolvedState,
    github: &GitHubSignals,
    kind: AgentKind,
    extra_agents: usize,
    age_ms: Option<u64>,
) -> String {
    let mut parts = Vec::new();
    if kind != AgentKind::Other {
        parts.push(kind.label().to_string());
    }
    if let Some(branch) = github.branch.as_ref().filter(|branch| !branch.is_empty()) {
        if !parts.iter().any(|part| part == branch) {
            parts.push(branch.clone());
        }
    }
    if let Some(age) = format_age(age_ms) {
        parts.push(age);
    }
    if extra_agents > 0 {
        parts.push(format!("+{extra_agents}"));
    }
    if parts.is_empty() {
        resolved.detail.clone()
    } else {
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmux::{
        AgentSnapshot, ScreenSnapshot, TabSnapshot, TerminalSnapshot, WorkspaceSnapshot,
    };

    fn snapshot_with_agent(state: &str, unread: bool) -> SessionSnapshot {
        SessionSnapshot {
            workspaces: vec![WorkspaceSnapshot {
                id: "ws_1".into(),
                name: "auth-refactor".into(),
                focused: false,
            }],
            screens: vec![ScreenSnapshot {
                id: "screen_1".into(),
                workspace_id: "ws_1".into(),
            }],
            panes: vec![crate::cmux::PaneSnapshot {
                id: "pane_1".into(),
                screen_id: "screen_1".into(),
            }],
            tabs: vec![TabSnapshot {
                id: "tab_1".into(),
                pane_id: "pane_1".into(),
                name: None,
                content_kind: Some("terminal".into()),
                content_id: Some("term_1".into()),
            }],
            terminals: vec![TerminalSnapshot {
                id: "term_1".into(),
                tab_ids: vec!["tab_1".into()],
                title: "claude".into(),
                cwd: None,
                running: true,
            }],
            agents: vec![AgentSnapshot {
                id: "agent_1".into(),
                terminal_id: "term_1".into(),
                state: state.into(),
                source: "hook".into(),
                updated_at_ms: Some(1_000),
                source_session: None,
            }],
            notifications: if unread {
                vec![NotificationSnapshot {
                    id: "n1".into(),
                    title: "Claude finished".into(),
                    body: String::new(),
                    level: "info".into(),
                    terminal_id: Some("term_1".into()),
                    created_at_ms: Some(2_000),
                    unread: true,
                }]
            } else {
                vec![]
            },
        }
    }

    #[test]
    fn running_agent_is_working() {
        let mut github = GitHubProbe::default();
        let tasks = build_tasks(&snapshot_with_agent("working", false), &mut github, 3_000);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].lane, AgentLane::Working);
        assert_eq!(tasks[0].kind, AgentKind::Claude);
    }

    #[test]
    fn unread_notification_promotes_working_agent() {
        let mut github = GitHubProbe::default();
        let tasks = build_tasks(&snapshot_with_agent("working", true), &mut github, 3_000);
        assert_eq!(tasks[0].lane, AgentLane::NeedsAttention);
    }
}
