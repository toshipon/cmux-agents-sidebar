//! Agent-kind detection. Kind is not on the public `AgentSnapshot`.

/// Known coding agents. `Other` covers unknown titles and missing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
    Pi,
    Other,
}

impl AgentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::Pi => "Pi",
            Self::Other => "Agent",
        }
    }

    fn from_token(token: &str) -> Option<Self> {
        match token {
            "claude" | "claude-code" | "claudecode" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "opencode" | "open-code" => Some(Self::OpenCode),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DetectInput<'a> {
    pub workspace_name: &'a str,
    pub tab_name: Option<&'a str>,
    pub terminal_title: Option<&'a str>,
    pub adapter_id: Option<&'a str>,
}

pub trait AgentDetector {
    fn detect(&self, input: &DetectInput<'_>) -> AgentKind;
}

/// Uses hook adapter ids when cmux starts exposing them. Harmless no-op today.
#[derive(Debug, Default, Clone, Copy)]
pub struct HookAgentDetector;

impl AgentDetector for HookAgentDetector {
    fn detect(&self, input: &DetectInput<'_>) -> AgentKind {
        input
            .adapter_id
            .and_then(|id| AgentKind::from_token(&id.to_ascii_lowercase()))
            .unwrap_or(AgentKind::Other)
    }
}

/// Same word split cmux-tui uses for `tabs.agents` title labels.
#[derive(Debug, Default, Clone, Copy)]
pub struct TitleAgentDetector;

impl AgentDetector for TitleAgentDetector {
    fn detect(&self, input: &DetectInput<'_>) -> AgentKind {
        for text in [
            Some(input.workspace_name),
            input.tab_name,
            input.terminal_title,
        ]
        .into_iter()
        .flatten()
        {
            if let Some(kind) = first_kind_in(text) {
                return kind;
            }
        }
        AgentKind::Other
    }
}

/// Tries hook metadata first, then titles. Process-name detectors can be
/// pushed on later without touching the resolver.
#[derive(Debug, Default, Clone)]
pub struct CompositeAgentDetector {
    hook: HookAgentDetector,
    title: TitleAgentDetector,
}

impl AgentDetector for CompositeAgentDetector {
    fn detect(&self, input: &DetectInput<'_>) -> AgentKind {
        let from_hook = self.hook.detect(input);
        if from_hook != AgentKind::Other {
            return from_hook;
        }
        self.title.detect(input)
    }
}

fn first_kind_in(text: &str) -> Option<AgentKind> {
    let lower = text.to_ascii_lowercase();
    lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .find_map(AgentKind::from_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_matches_cmux_default_agents() {
        let detector = TitleAgentDetector;
        assert_eq!(
            detector.detect(&DetectInput {
                workspace_name: "auth",
                terminal_title: Some("claude --resume abc"),
                ..DetectInput::default()
            }),
            AgentKind::Claude
        );
        assert_eq!(
            detector.detect(&DetectInput {
                workspace_name: "pay",
                terminal_title: Some("0 codex"),
                ..DetectInput::default()
            }),
            AgentKind::Codex
        );
        assert_eq!(
            detector.detect(&DetectInput {
                workspace_name: "app",
                tab_name: Some("opencode"),
                ..DetectInput::default()
            }),
            AgentKind::OpenCode
        );
    }

    #[test]
    fn unknown_title_is_other() {
        let detector = TitleAgentDetector;
        assert_eq!(
            detector.detect(&DetectInput {
                workspace_name: "notes",
                terminal_title: Some("zsh"),
                ..DetectInput::default()
            }),
            AgentKind::Other
        );
    }

    #[test]
    fn hook_adapter_wins_over_title() {
        let detector = CompositeAgentDetector::default();
        assert_eq!(
            detector.detect(&DetectInput {
                workspace_name: "zsh",
                terminal_title: Some("zsh"),
                adapter_id: Some("codex"),
                ..DetectInput::default()
            }),
            AgentKind::Codex
        );
    }
}
