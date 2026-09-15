//! Minimal `cmux.protocol/2` client for snapshot + focus.
//!
//! Implemented from the public catalog (`cmux-tui/spec/resource-api-v2.md`).
//! Unknown JSON fields are ignored so newer muxes stay readable.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const PROTOCOL: &str = "cmux.protocol/2";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceSnapshot {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub focused: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScreenSnapshot {
    pub id: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaneSnapshot {
    pub id: String,
    pub screen_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TabSnapshot {
    pub id: String,
    pub pane_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub content_kind: Option<String>,
    #[serde(default)]
    pub content_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TerminalSnapshot {
    pub id: String,
    #[serde(default)]
    pub tab_ids: Vec<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub running: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotificationSnapshot {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub terminal_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    pub created_at_ms: Option<u64>,
    #[serde(default)]
    pub unread: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentSnapshot {
    pub id: String,
    pub terminal_id: String,
    pub state: String,
    #[serde(default)]
    pub source: String,
    #[serde(default, deserialize_with = "deserialize_opt_u64")]
    pub updated_at_ms: Option<u64>,
    #[serde(default)]
    pub source_session: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionSnapshot {
    #[serde(default)]
    pub workspaces: Vec<WorkspaceSnapshot>,
    #[serde(default)]
    pub screens: Vec<ScreenSnapshot>,
    #[serde(default)]
    pub panes: Vec<PaneSnapshot>,
    #[serde(default)]
    pub tabs: Vec<TabSnapshot>,
    #[serde(default)]
    pub terminals: Vec<TerminalSnapshot>,
    #[serde(default)]
    pub notifications: Vec<NotificationSnapshot>,
    #[serde(default)]
    pub agents: Vec<AgentSnapshot>,
}

pub struct CmuxSession {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
    pub socket_path: PathBuf,
}

impl CmuxSession {
    pub fn connect(socket_path: impl AsRef<Path>) -> Result<Self> {
        let socket_path = socket_path.as_ref().to_path_buf();
        let stream = UnixStream::connect(&socket_path)
            .with_context(|| format!("cannot connect to {}", socket_path.display()))?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            next_id: 1,
            socket_path,
        })
    }

    pub fn snapshot(&mut self) -> Result<SessionSnapshot> {
        let result = self.read("session.snapshot", current_session_params())?;
        serde_json::from_value(result).context("session.snapshot shape")
    }

    pub fn focus_workspace(&mut self, workspace_id: &str) -> Result<()> {
        let mut params = current_session_params();
        params.insert("workspace".into(), json!(workspace_id));
        self.mutate("workspace.focus", params)?;
        Ok(())
    }

    pub fn focus_tab(&mut self, tab_id: &str) -> Result<()> {
        let mut params = current_session_params();
        params.insert("tab".into(), json!(tab_id));
        self.mutate("tab.focus", params)?;
        Ok(())
    }

    fn read(&mut self, operation: &str, params: Map<String, Value>) -> Result<Value> {
        self.round_trip(operation, params, None)
    }

    fn mutate(&mut self, operation: &str, params: Map<String, Value>) -> Result<Value> {
        self.round_trip(operation, params, Some(fresh_idempotency_key()))
    }

    fn round_trip(
        &mut self,
        operation: &str,
        params: Map<String, Value>,
        idempotency_key: Option<String>,
    ) -> Result<Value> {
        let id = format!("req-{}", self.next_id);
        self.next_id += 1;
        let mut envelope = json!({
            "protocol": PROTOCOL,
            "type": "request",
            "id": id,
            "operation": operation,
            "params": params,
        });
        if let Some(key) = idempotency_key {
            envelope["idempotency_key"] = json!(key);
        }
        let mut line = serde_json::to_vec(&envelope)?;
        line.push(b'\n');
        self.stream.write_all(&line)?;
        self.stream.flush()?;

        let started = Instant::now();
        loop {
            if started.elapsed() > REQUEST_TIMEOUT {
                return Err(anyhow!("{operation} timed out"));
            }
            let mut response_line = String::new();
            let n = self.reader.read_line(&mut response_line)?;
            if n == 0 {
                return Err(anyhow!("cmux socket closed"));
            }
            let value: Value =
                serde_json::from_str(response_line.trim()).context("invalid JSON from cmux")?;
            if value.get("event").is_some()
                || value.get("type").and_then(Value::as_str) == Some("event")
            {
                continue;
            }
            return decode_response(value, &id);
        }
    }
}

fn current_session_params() -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("machine".into(), json!("current"));
    params.insert("session".into(), json!("current"));
    params
}

fn decode_response(value: Value, expected_id: &str) -> Result<Value> {
    if value.get("protocol").and_then(Value::as_str) != Some(PROTOCOL) {
        return Err(anyhow!("expected cmux.protocol/2 response"));
    }
    if value.get("id").and_then(Value::as_str) != Some(expected_id) {
        return Err(anyhow!("response id mismatch"));
    }
    match value.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
        Some(false) => {
            let error = value.get("error").cloned().unwrap_or(Value::Null);
            let code = error.get("code").and_then(Value::as_str).unwrap_or("error");
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("cmux error");
            Err(anyhow!("{code}: {message}"))
        }
        None => Err(anyhow!("response missing ok")),
    }
}

fn fresh_idempotency_key() -> String {
    let mut bytes = [0_u8; 16];
    getrandom_fill(&mut bytes);
    let mut key = String::from("agents-");
    for byte in bytes {
        key.push_str(&format!("{byte:02x}"));
    }
    key
}

fn getrandom_fill(bytes: &mut [u8]) {
    if let Ok(n) = std::fs::read("/dev/urandom") {
        for (dst, src) in bytes.iter_mut().zip(n.iter()) {
            *dst = *src;
        }
        if bytes.iter().any(|b| *b != 0) {
            return;
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = ((nanos >> ((i % 8) * 8)) & 0xff) as u8 ^ (i as u8).wrapping_mul(17);
    }
}

fn deserialize_opt_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(match value {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.parse().ok(),
        Some(_) => None,
    })
}

pub fn socket_from_env() -> Option<PathBuf> {
    std::env::var_os("CMUX_TUI_SOCKET")
        .or_else(|| std::env::var_os("CMUX_MUX_SOCKET"))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[derive(Debug, Clone)]
pub struct TerminalRef {
    pub workspace_id: String,
    pub tab_id: Option<String>,
    pub title: String,
    pub tab_name: Option<String>,
    pub cwd: Option<PathBuf>,
}

pub fn index_terminals(snapshot: &SessionSnapshot) -> BTreeMap<String, TerminalRef> {
    let screens: BTreeMap<&str, &str> = snapshot
        .screens
        .iter()
        .map(|screen| (screen.id.as_str(), screen.workspace_id.as_str()))
        .collect();
    let panes: BTreeMap<&str, &str> = snapshot
        .panes
        .iter()
        .map(|pane| (pane.id.as_str(), pane.screen_id.as_str()))
        .collect();
    let tabs: BTreeMap<&str, &TabSnapshot> = snapshot
        .tabs
        .iter()
        .map(|tab| (tab.id.as_str(), tab))
        .collect();
    let tab_by_terminal: BTreeMap<&str, &TabSnapshot> = snapshot
        .tabs
        .iter()
        .filter_map(|tab| {
            tab.content_id
                .as_deref()
                .or_else(|| {
                    if tab.content_kind.as_deref() == Some("terminal") {
                        tab.content_id.as_deref()
                    } else {
                        None
                    }
                })
                .map(|content| (content, tab))
        })
        .collect();

    let mut index = BTreeMap::new();
    for terminal in &snapshot.terminals {
        let tab = terminal
            .tab_ids
            .iter()
            .find_map(|id| tabs.get(id.as_str()).copied())
            .or_else(|| tab_by_terminal.get(terminal.id.as_str()).copied());
        let workspace_id = tab
            .and_then(|tab| panes.get(tab.pane_id.as_str()).copied())
            .and_then(|screen_id| screens.get(screen_id).copied())
            .unwrap_or("")
            .to_string();
        index.insert(
            terminal.id.clone(),
            TerminalRef {
                workspace_id,
                tab_id: tab.map(|tab| tab.id.clone()),
                title: terminal.title.clone(),
                tab_name: tab.and_then(|tab| tab.name.clone()),
                cwd: terminal.cwd.as_ref().map(PathBuf::from),
            },
        );
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_agent_terminal_into_workspace() {
        let snapshot = SessionSnapshot {
            workspaces: vec![WorkspaceSnapshot {
                id: "ws_1".into(),
                name: "auth".into(),
                focused: true,
            }],
            screens: vec![ScreenSnapshot {
                id: "screen_1".into(),
                workspace_id: "ws_1".into(),
            }],
            panes: vec![PaneSnapshot {
                id: "pane_1".into(),
                screen_id: "screen_1".into(),
            }],
            tabs: vec![TabSnapshot {
                id: "tab_1".into(),
                pane_id: "pane_1".into(),
                name: Some("claude".into()),
                content_kind: Some("terminal".into()),
                content_id: Some("term_1".into()),
            }],
            terminals: vec![TerminalSnapshot {
                id: "term_1".into(),
                tab_ids: vec!["tab_1".into()],
                title: "claude".into(),
                cwd: Some("/tmp/auth".into()),
                running: true,
            }],
            ..SessionSnapshot::default()
        };
        let index = index_terminals(&snapshot);
        let term = index.get("term_1").expect("terminal");
        assert_eq!(term.workspace_id, "ws_1");
        assert_eq!(term.tab_id.as_deref(), Some("tab_1"));
        assert_eq!(term.cwd.as_deref(), Some(Path::new("/tmp/auth")));
    }
}
