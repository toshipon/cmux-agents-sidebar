# cmux-agents-sidebar

Attention-first sidebar plugin for [cmux](https://github.com/manaflow-ai/cmux).

This is not a workspace switcher. It treats cmux as a **control plane for many agents**: look at the sidebar, then only visit the sessions that need a human.

```text
        Human
          │
          ▼
┌─────────────────────┐
│ cmux Agents Sidebar │
│                     │
│ ⚠ Attention      2  │
│ ◉ Review         3  │
│ ● Working        7  │
└─────────┬───────────┘
          │
   ┌──────┼──────┐
   ▼      ▼      ▼
Claude  Codex  OpenCode
```

## ASCII preview

```text
AGENTS

▾ NEEDS ATTENTION  2
 ⚠ auth-refactor
   Claude · 2m
   Waiting for input
 ⚠ terraform-prod
   Codex · 30s
   Permission required

▾ WORKING  3
 ● payment-api
   feature/payment · 8m
   Working
 ● frontend
   fix/dashboard · 4m
   Working

▾ IN REVIEW  2
 ◉ auth-api  #184
   Claude · feature/auth
   ✓ CI · Review pending

▾ DONE  1
 ✓ renovate  #179
   Merged

▾ IDLE  1
 ○ notes
   Idle
```

## Concept

First-class objects are **Task / Agent → Status → Workspace**, not pane trees.

Priority:

```text
NeedsAttention > InReview > Working > Idle
```

`Done` sits at the bottom. An agent that is still running but posted an unread “finished, please check” notification moves to **Needs Attention**.

Lane inference is a pure `StateResolver`. The TUI never classifies state itself.

## Requirements

- [cmux-tui](https://github.com/manaflow-ai/cmux/tree/main/cmux-tui) with sidebar plugins and `cmux.protocol/2`
- Rust 1.88+ to build from source
- Optional: `gh` and `git` for PR / CI / review lanes

This plugin talks to **cmux-tui**, not the macOS Ghostty cmux app. The macOS app already has a built-in workspace status glyph.

## Installation

Plugin support must be present in your cmux-tui build.

```sh
cmux sidebar plugin install https://github.com/toshipon/cmux-agents-sidebar
cmux sidebar plugin use agents
```

If a session is already running:

```sh
cmux server reload-config
```

Focus the sidebar with the usual chord (`Ctrl-b S` by default).

### Return to the built-in sidebar

```sh
cmux sidebar plugin use --builtin
cmux server reload-config
```

### Standalone development

```sh
CMUX_TUI_SOCKET=/path/to/cmux-tui.sock cargo run
```

Socket paths are usually `$XDG_RUNTIME_DIR/cmux-tui-<uid>/main.sock` or `/tmp/cmux-tui-<uid>/main.sock`. Missing `CMUX_TUI_SOCKET` shows a reconnect screen instead of panicking.

## Configuration

No plugin config file is required. Classification uses:

| Source | Interval |
| --- | --- |
| `session.snapshot` over `CMUX_TUI_SOCKET` | 1s |
| `gh pr view --json` per git cwd | 20s cache, at most 2 fetches per tick |
| key input | 100ms poll |

Reconnect backoff matches the official fzf plugin (500ms … 8s).

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| `↑` `↓` / `Ctrl-p` `Ctrl-n` / `Ctrl-k` `Ctrl-j` | Move selection |
| `Enter` | Jump to the workspace (and agent tab when known) |
| `Tab` / `Shift-Tab` | Next / previous group |
| `Space` | Collapse or expand the current group |
| `r` | Refresh immediately |
| `q` / `Ctrl-c` | Quit |
| `Esc` | Ignored (cmux owns the prefix escape chord) |

Mouse input is not forwarded to sidebar plugins by cmux.

## GitHub integration

Optional. The sidebar works without GitHub.

When `gh` is on `PATH` and a workspace terminal has a git cwd, the plugin runs:

```sh
gh pr view --json number,state,isDraft,reviewDecision,statusCheckRollup,mergedAt,headRefName,url,title
```

| Signal | Lane effect |
| --- | --- |
| Open PR, agent not working | **In Review** |
| Merged PR, agent idle/done | **Done** |
| `gh` missing, not a repo, no PR, auth/network error | Ignored; cmux agent state still applies |

## Status model

cmux-tui agent states (`working`, `blocked`, `idle`, `done`, `unknown`) plus unread notifications plus optional PR metadata:

| Inputs | Lane |
| --- | --- |
| `blocked` or unread notification | Needs Attention |
| Open PR and agent not `working` | In Review |
| Agent `working` | Working |
| Merged PR or agent `done` | Done |
| Otherwise | Idle |

`blocked` is how cmux-tui reports permission prompts, questions, and plan review (not a process-name heuristic).

Agent kind (Claude / Codex / OpenCode / Pi) is inferred from tab/terminal titles the same way cmux-tui labels tabs. The public `AgentSnapshot` does not currently include adapter id; see `docs/upstream-proposal.md`.

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| Reconnecting forever | `CMUX_TUI_SOCKET` unset, or mux not running |
| Empty IDLE-only list | Agents have not reported yet. Install hooks: `cmux agent hook install` |
| Everything is Agent, not Claude/Codex | Rename is custom and title has no `claude`/`codex`/`opencode`/`pi` token |
| No In Review | `gh` not installed, not logged in, cwd is not a git checkout, or no PR on the branch |
| Jump does nothing | Socket dropped; plugin will reconnect. Enter retries after refresh |
| Plugin crash-loops | cmux backs off restarts. Run standalone with the socket env to see the error |
| Built-in sidebar still showing | `cmux sidebar plugin use agents` then `cmux server reload-config` |

## Architecture

```text
cmux.protocol/2  session.snapshot
        │
        ├── workspace / screen / pane / tab / terminal
        ├── agent (working|blocked|idle|done)
        └── notification (unread)
                │
                ▼
          StateResolver  ← optional gh PR JSON
                │
                ▼
          grouped TUI
```

| Module | Role |
| --- | --- |
| `cmux.rs` | Unix JSON-lines `cmux.protocol/2` client |
| `state.rs` | Pure lane inference |
| `agent.rs` | `AgentDetector` trait (title + future hook id) |
| `github.rs` | Cached `gh` probe |
| `model.rs` | Per-workspace aggregation |
| `ui.rs` | Ratatui view |

Design notes: `docs/research.md`, `docs/decisions.md`.

## cmux compatibility

| API | Used |
| --- | --- |
| `CMUX_TUI_SOCKET` / `CMUX_MUX_SOCKET` | Yes |
| `cmux.protocol/2` `session.snapshot` | Yes |
| `workspace.focus` / `tab.focus` | Yes |
| `agent.list` fields via snapshot | Yes |
| Unread notifications | Yes |
| crates.io `cmux-client` 0.1 (protocol v12) | No — needs typed agents + cwd |
| macOS `sidebar-state` / `list-status` / `surface-health` | No — different product |

Tested against the cmux-tui catalog in manaflow-ai/cmux (protocol/2 `AgentSnapshot`, `NotificationSnapshot`).

## Build / test

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

## License

MIT, same as [cmux-sidebar-fzf](https://github.com/manaflow-ai/cmux-sidebar-fzf). This repository does not vendor GPL cmux sources.
