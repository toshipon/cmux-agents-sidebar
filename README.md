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

PINNED  1
 📌 checkout-flow
   Claude · Working

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

Pinned workspaces stay in a **PINNED** block at the top of the macOS sidebar and are omitted from the status lanes so they are not listed twice.

Lane inference is a pure `StateResolver`. The TUI never classifies state itself.

## Two products named cmux

| You ran | What you have | Install |
| --- | --- | --- |
| `cmux sidebar plugin` → `Unknown sidebar command 'plugin'` | **macOS cmux app** CLI (`validate` / `reload` / `select` / `open` only) | Custom sidebar JS below |
| `cmux sidebar plugin` is a real subcommand | **cmux-tui** | PTY plugin further down |

If you are on a Mac and installed cmux as a desktop app, you are on the first row. `sidebar plugin` cannot be made to exist from this repository.

## Requirements

- macOS cmux app (custom sidebars beta, on by default), **or**
- [cmux-tui](https://github.com/manaflow-ai/cmux/tree/main/cmux-tui) with sidebar plugins and `cmux.protocol/2`
- Rust 1.88+ only if you build the tui plugin from source
- Optional for tui: `gh` and `git` for PR / CI / review lanes. macOS uses live `w.pr` from the app instead.

## Installation (macOS cmux app)

Custom sidebars are files in `~/.config/cmux/sidebars`. The filename without extension is the sidebar name.

From a clone of this repo:

```sh
./scripts/install-macos.sh
cmux sidebar validate agents
cmux sidebar select agents
```

Or by hand:

```sh
mkdir -p ~/.config/cmux/sidebars
curl -fsSL https://raw.githubusercontent.com/toshipon/cmux-agents-sidebar/main/sidebars/agents.js \
  -o ~/.config/cmux/sidebars/agents.js
cmux sidebar validate agents
cmux sidebar select agents
```

Other ways to show it:

```sh
cmux sidebar open agents                 # Bonsplit pane
cmux right-sidebar set custom agents     # right panel
```

Right-click the sidebar toggle and choose **agents**. Edit the file and save; it hot-reloads. Turn custom sidebars off in **Settings → Custom Sidebars** if the option is missing.

The JS runtime cannot spawn `gh`. Open PRs still land in **In Review** when cmux already attached `w.pr`.

### Return to the built-in sidebar

There is no `cmux sidebar plugin use --builtin` on the macOS app.

If you used `cmux sidebar select agents` (left sidebar):

1. Right-click the sidebar toggle button
2. Choose **Default Workspaces**

That is the built-in tree with workspace groups.

If you used `cmux sidebar open agents`, close that pane tab. The left sidebar is unchanged.

If you used the right panel:

```sh
cmux right-sidebar set files
```

You can keep `~/.config/cmux/sidebars/agents.js`. Switching the picker does not delete the file.

## Installation (cmux-tui)

Plugin support must be present in your cmux-tui build. These commands fail on the macOS app CLI.

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
| macOS `w.pr` present | Same lane effects without spawning `gh` |

## Status model

cmux-tui agent states (`working`, `blocked`, `idle`, `done`, `unknown`) plus unread notifications plus optional PR metadata. macOS maps `needs_input` → blocked and `ended` → done, and uses `w.pr` instead of `gh`.

| Inputs | Lane |
| --- | --- |
| `blocked` / `needs_input` or unread notification | Needs Attention |
| Open PR and agent not `working` | In Review |
| Agent `working` | Working |
| Merged PR or agent `done` / `ended` | Done |
| Otherwise | Idle |

`blocked` / `needs_input` is how cmux reports permission prompts, questions, and plan review (not a process-name heuristic).

On macOS, agent kind comes from `workspaces[i].agents[j].kind`. On cmux-tui it is inferred from tab/terminal titles; the public `AgentSnapshot` does not currently include adapter id. See `docs/upstream-proposal.md`.

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| `Unknown sidebar command 'plugin'` | You are on the macOS app. Use Installation (macOS cmux app) |
| Custom sidebar missing after copy | Settings → Custom Sidebars enabled; `cmux sidebar validate agents` |
| Reconnecting forever | `CMUX_TUI_SOCKET` unset, or mux not running |
| Empty IDLE-only list | Agents have not reported yet. Install hooks: `cmux agent hook install` |
| Everything is Agent, not Claude/Codex | Rename is custom and title has no `claude`/`codex`/`opencode`/`pi` token |
| No In Review | tui: `gh` missing/not a repo. macOS: workspace has no `pr` yet |
| Jump does nothing | Socket dropped; plugin will reconnect. Enter retries after refresh |
| Plugin crash-loops | cmux backs off restarts. Run standalone with the socket env to see the error |
| Want the built-in tree back | Right-click the sidebar toggle → **Default Workspaces** (macOS). tui: `plugin use --builtin` |

## Architecture

```text
cmux-tui                         macOS cmux app
cmux.protocol/2 snapshot         live workspaces[].agents
        │                                │
        ▼                                ▼
  StateResolver (Rust)            same lanes in sidebars/agents.js
        │                                │
        ▼                                ▼
     Ratatui TUI                    SwiftUI custom sidebar
```

| Module | Role |
| --- | --- |
| `cmux.rs` | Unix JSON-lines `cmux.protocol/2` client |
| `state.rs` | Pure lane inference |
| `agent.rs` | `AgentDetector` trait (title + future hook id) |
| `github.rs` | Cached `gh` probe |
| `model.rs` | Per-workspace aggregation |
| `ui.rs` | Ratatui view |
| `sidebars/agents.js` | macOS custom sidebar (same lanes, live `workspaces`) |

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
| macOS `~/.config/cmux/sidebars/agents.js` | Yes — custom sidebar, `workspace.select` / `surface.focus` |
| macOS `sidebar-state` / `list-status` / `surface-health` | No — JS binds live `workspaces` instead |

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
