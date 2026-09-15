# Research: cmux Agents Sidebar

調査日: 2026-09-15  
対象:

- [manaflow-ai/cmux](https://github.com/manaflow-ai/cmux) `main` (clone of latest)
- [manaflow-ai/cmux-sidebar-fzf](https://github.com/manaflow-ai/cmux-sidebar-fzf) `main`
- crates.io `cmux-client` 0.1.2 / `cmux-sdk` 0.0.0-bootstrap.0

README だけでなく、`cmux-tui` の spec・実装・公式 plugin・macOS アプリ側の workspace status 推論まで確認した。

---

## 0. 重要な前提: cmux は二系統ある

調査中に、依頼文中の API 名が **二つの別プロダクト** にまたがっていることが分かった。

| 系統 | 実体 | Sidebar plugin | 依頼中の API |
| --- | --- | --- | --- |
| **cmux-tui** | Rust の terminal multiplexer (`cmux-tui/`) | **これが対象**。`cmux-plugin.toml` + PTY plugin | `CMUX_TUI_SOCKET`, `cmux sidebar plugin install/use`, `agent.list`, `session.snapshot` |
| **macOS cmux app** | Ghostty ベースの Swift アプリ (`Sources/`, `CLI/`) | 公式 built-in sidebar。plugin 契約ではない | `sidebar-state --json`, `list-status`, `set-progress`, `surface-health`, `workspace status set` |

公式 reference plugin `cmux-sidebar-fzf` は **cmux-tui 専用**。インストールコマンドも cmux-tui 側:

```bash
cmux sidebar plugin install https://github.com/manaflow-ai/cmux-sidebar-fzf
cmux sidebar plugin use fzf
cmux sidebar plugin use --builtin
```

macOS アプリはすでに Cursor Agents に近い **workspace task status** (`todo / working / needs-attention / review / done`) を built-in sidebar で持っている。cmux-tui 側は Ratatui plugin で同じレーンを再現する。macOS 側は `cmux sidebar plugin` が存在しないため、同じレーンを `sidebars/agents.js`（custom sidebar）として別途載せる。

本リポジトリは cmux 本体を fork しない。tui は Sidebar Plugin、macOS は `~/.config/cmux/sidebars` の公式 custom-sidebar 契約で独立する。

---

## 1. Sidebar plugin contract (cmux-tui)

根拠: `cmux-tui/spec/plugins.md`, `cmux-tui/docs/configuration.md`, `cmux-tui/crates/cmux-tui/src/plugin_manager.rs`, `cmux-sidebar-fzf`.

### Lifecycle

- plugin は普通の TUI プログラム。cmux が sidebar 矩形の PTY で起動し、出力を sidebar に描画する。
- 初回表示時に起動。sidebar を隠しても **kill しない**。mux server 終了、または config の plugin command 変更で kill。
- 起動失敗時は sidebar にエラーを出し、bounded restart backoff。crash loop しない。
- 環境変数:
  - `CMUX_TUI_SOCKET` — JSON-lines control socket（正）
  - `CMUX_MUX_SOCKET` — legacy alias
  - `CMUX_SIDEBAR=1`
  - `TERM` — 通常 PTY と同じ
- リサイズは通常の PTY `SIGWINCH`。plugin 固有プロトコルはない。
- **Mouse は plugin に転送されない**（この round）。クリックは focus するだけ。
- **Esc で exit してはいけない**。cmux の prefix chord が escape hatch。`cmux-sidebar-fzf` は Esc で query clear、Ctrl-C で clean exit。

### Manifest (`cmux-plugin.toml`)

```toml
[plugin]
name = "fzf"          # [a-z0-9-_]+
kind = "sidebar"
version = "0.1.0"
description = "..."

[run]
command = ["target/release/cmux-sidebar-fzf"]

[build]
command = ["cargo", "build", "--release"]
```

Install は git clone → manifest 検証 → `[build].command` → `[run].command[0]` が executable か確認 → `~/.local/share/cmux/mux-plugins/<name>`。`sidebar plugin use` が `sidebar.plugin.command` を絶対パスで config に書く。running session は `cmux server reload-config` が別途必要。

### 公式 plugin の技術選択

`cmux-sidebar-fzf`:

- Rust edition 2024, MSRV 1.88, MIT
- `cmux-client = "0.1"` (crates.io, **旧 protocol-v12**)
- `ratatui 0.30` + `crossterm 0.29`
- 2 秒ポーリングで `list_workspaces`
- reconnect backoff 500ms → 8s
- jump: `select_workspace` / `select_screen` / `focus_pane`

---

## 2. Control socket / CLI / protocol

cmux-tui には **二つの並行プロトコル** がある。

### 2.1 旧 protocol (v12 / raw JSON-lines)

`{"id":1,"cmd":"list-workspaces"}` 形式。`cmux-client` 0.1.2 がこれを話す。

実装済みで本 plugin に関係するもの (`cmux-tui/spec/commands.md`):

| Command | 用途 |
| --- | --- |
| `identify` | 接続確認 |
| `list-workspaces` | workspace / screen / pane / tab tree |
| `select-workspace` / `select-screen` / `focus-pane` | jump |
| `list-agents` | agent records (`working\|blocked\|idle\|done\|unknown`) |
| `report-agent` | 明示レポート |
| `subscribe` | tree-changed, notification, … |
| `notify` | 通知投稿 |
| `process-info` | pid / command / cwd / foreground_cwd |

`agent-state-changed` は **proposed vNext** で、subscribe にはまだ載っていない (`spec/events.md`)。

### 2.2 公開 `cmux.protocol/2`（推奨）

Unix socket 上の JSON object / line。envelope:

```json
{
  "protocol": "cmux.protocol/2",
  "type": "request",
  "id": "req-1",
  "operation": "session.snapshot",
  "params": { "machine": "current", "session": "current" }
}
```

成功:

```json
{ "protocol": "cmux.protocol/2", "type": "response", "id": "req-1", "ok": true, "result": { } }
```

mutation は `idempotency_key` 必須。CLI は `cmux <resource> <action>`。同一 socket が raw command と protocol/2 の両方を受け付ける。

公式 SDK は `cmux-tui/bindings/rust`（crate 名 `cmux` / package `cmux-sdk`）。ただし:

- crates.io `cmux-sdk` は `0.0.0-bootstrap.0` で **未公開**
- monorepo ルートは Cargo workspace ではないため、git dependency で `cmux-sdk` を引くのは困難
- 公式 consumer 例 `bindings/examples/rust-agent-dashboard` は **path 依存 + `session.snapshot` を 1 秒ポーリング**

### 2.3 `session.snapshot` が一次情報源

`ResourceSnapshot` は **1 回の read** で次を返す:

- workspaces, screens, panes, tabs
- terminals (`title`, `cwd`, lifecycle)
- notifications (`unread`, `level`, `terminal_id`, `created_at_ms`)
- agents (`state`, `source`, `terminal_id`, `updated_at_ms`, `source_session`)
- cursor / revision

rust-agent-dashboard の FRICTION.md: 「One `session.snapshot` call replaces five filtered agent queries」。

### 2.4 依頼文中の macOS CLI について

次は **macOS アプリ CLI** (`docs/cli-contract.md`) であり、cmux-tui sidebar plugin からは使えない:

- `sidebar-state --json`
- `list-status` / `set-status` / `set-progress`
- `surface-health`
- `list-notifications`（macOS 側。tui 側は `notification.list` / snapshot）
- `workspace status set todo|working|needs-attention|review|done|auto`

対応関係:

| macOS | cmux-tui |
| --- | --- |
| `list-workspaces` | `workspace.list` / snapshot |
| `list-notifications --json` | `notification.list` / snapshot.notifications |
| `workspace status` (inferred lanes) | **plugin 側 StateResolver が同等を計算** |
| `surface-health` | なし（terminal.lifecycle が近似） |
| `set-status` pills | なし（AgentSnapshot.state が正） |

---

## 3. Agent state: 公式 API で取れるもの / 取れないもの

### 3.1 取得可能（推測ではない）

`AgentSnapshot` (`spec/resource-operations-v2.json`, `mux.rs`):

```text
id, session_id, terminal_id,
state: working | blocked | idle | done | unknown,
source: hook | socket | detected,
updated_at_ms,
source_session: string | null   // agent 自身の session id。provider 名ではない
```

hook journal からの正規化 (`mux.rs` `agent_state_for_hook_kind`):

| hook kind | AgentState | 通知 |
| --- | --- | --- |
| `agent.session.started` / `agent.turn.completed` | Idle | turn.completed → "finished" (info) |
| `agent.turn.started` | Working | なし |
| `agent.approval.requested` / `question` / `plan_review` / `error` | **Blocked** | warning/error |
| `agent.session.ended` | Done | なし |

つまり **「実行中」と「人間の入力待ち」は公式 API で区別できる**。入力待ちは macOS の `needsInput` ではなく、tui では **`blocked`**。

通知:

- `unread: bool`
- `level: info | warning | error`
- `terminal_id` で workspace に紐付け可能
- turn 完了は info の "finished" 通知になる。これを読めば「確認してください」を NeedsAttention に上げられる。

Workspace 紐付け:

```
agent.terminal_id → terminal.tab_ids → tab.pane_id → pane.screen_id → screen.workspace_id
```

cwd: `TerminalSnapshot.cwd`（GitHub 用）。`terminal.process.get` で live `foreground_cwd` / argv も取れるが、snapshot の cwd を優先し N+1 を避ける。

### 3.2 取得不可能

| 欲しい情報 | 現実 |
| --- | --- |
| Agent が Claude / Codex / OpenCode / Pi のどれか | **AgentSnapshot に provider / adapter.id が無い**。adapter.id は journal payload (`payload.adapter.id`) にだけある。 |
| macOS の `needsInput` という名前 | tui では `blocked` |
| PR number / CI / review | **cmux-tui に GitHub API は無い**。macOS sidebar はアプリ内蔵。 |
| `sidebar-state` の pills / progress | tui に相当 API なし |
| subscribe 上の `agent-state-changed` | proposed のみ。v2 は `session.events` の resource upsert |
| プロセス名による公式 agent detection | `AgentSource::Detected` は enum 値として存在するが、現行 core に検出器実装はほぼ無く、hook/socket report が正。タイトルからの agent 名抽出は **TUI 表示用** (`config.rs` `agent_in_title`: デフォルト `["claude","codex","opencode","pi"]`) |

### 3.3 Workaround

1. **Agent kind**: `TitleAgentDetector`（cmux-tui と同じ title word match）。将来 journal `adapter.id` が snapshot に載ったら `HookAgentDetector` に差し替え。process name には強く依存しない。
2. **NeedsAttention**: `state == blocked` **または** unread notification。
3. **InReview / Done (PR)**: optional `gh` CLI。失敗しても sidebar は落ちない。
4. **更新**: `session.snapshot` 1s ポーリング（公式 dashboard と同じ）。`session.events` は専用接続が要るため MVP では使わない。

### 3.4 cmux 本体への変更が必要か

**必須ではない。** 現行 API で MVP は成立する。

綺麗になる追加（Upstream Proposal、本 plugin とは非結合）:

1. `AgentSnapshot.adapter_id`（hook の `payload.adapter.id`）
2. subscribe / `session.events` の安定した `agent-changed` を sidebar 向けに文書化
3. macOS と揃えた lane 名 (`needs-attention`) を public agent state に載せるかは議論（今は plugin が map すれば足りる）

---

## 4. GitHub

cmux-tui に PR metadata は無い。macOS は `sidebar.showPullRequests` でアプリが取る。

本 plugin は optional `gh`:

```bash
gh pr view --json number,state,isDraft,reviewDecision,statusCheckRollup,mergedAt,headRefName,url,title
```

cwd は terminal snapshot。git でない / `gh` 未install / 未login / PR なし / 通信失敗 → GitHub 信号なしとして StateResolver に渡す。

ポーリング: cwd 単位で 20s キャッシュ。連続 spawn しない。

---

## 5. Agent detection

macOS: hook store (`~/.cmuxterm/<agent>-hook-sessions.json`) に agent 名と lifecycle (`running|idle|needsInput|unknown`)。

cmux-tui: `cmux agent hook install` が Codex / Claude / Gemini / OpenCode / Pi 等に helper を入れる。正規化後の **公開投影には adapter 名が落ちる**。

TUI 側の既存ヒューリスティック: tab title を word split し `tabs.agents` と照合。デフォルト 4 種。

本 plugin: `AgentDetector` trait。

- `TitleAgentDetector` — 公式 TUI と同じ
- `CompositeAgentDetector` — 将来 Hook / Process を追加
- process name 単体への強い依存はしない

---

## 6. Status model の根拠（macOS 実装）

`Packages/macOS/CmuxWorkspaces/.../WorkspaceTaskStatus.swift` は検証済みの推論である:

```text
anyAgentNeedsInput → needs-attention
anyAgentRunning    → working
anyOpenPullRequest → review
all PRs merged/closed → done
dirty git          → working
else               → todo
```

first match wins。本 plugin はこれを cmux-tui 信号に写す:

```text
blocked OR unread notification → NeedsAttention
open PR (agent idle)           → InReview
working                        → Working
merged PR (agent idle/done)    → Done
else                           → Idle
```

依頼の優先順位 `NeedsAttention > InReview > Working > Idle` と一致。Done は別グループ（mock 通り下部）。

dirty git → Working は MVP では入れない（`git status` の連続 spawn を避ける）。将来 `WaitingCI` 等を `AgentLane` に追加できるよう非網羅 match にしない。

---

## 7. Performance / reliability

| ソース | 間隔 | 根拠 |
| --- | --- | --- |
| `session.snapshot` | 1s | rust-agent-dashboard デフォルト。CLI spawn なし |
| key poll | 100ms | fzf plugin |
| GitHub `gh` | 20s / cwd | 低頻度。失敗はキャッシュ |
| reconnect | 500ms…8s | fzf plugin |
| `session.events` | MVP 対象外 | 専用接続。dashboard も未使用 |

壊れても落ちない: socket disconnect, workspace/surface 消滅, gh なし, 非 git, PR なし, unknown agent, notification 欠落。

---

## 8. License

- cmux 本体: GPL-3.0-or-later
- cmux-tui workspace / SDK / sidebar-fzf: **MIT**
- 本 plugin は fzf に合わせて **MIT**。GPL の cmux ソースはコピーしない。protocol は公開 spec に基づき自前実装。

---

## 9. Keyboard

cmux 公式 sidebar (built-in): Up/Down, Space toggle, Enter activate。plugin は PTY にキーがそのまま来る。prefix は cmux が奪う。

本 plugin:

| Key | Action |
| --- | --- |
| ↑ ↓ / Ctrl-p n k j | 行選択（fzf と同じ） |
| Enter | workspace + tab へ focus |
| Tab | 次グループ |
| Space | グループ collapse |
| r | 即 refresh |
| q / Ctrl-C | quit |
| Esc | no-op（cmux の chord を妨げない） |
