# Design decisions

Each decision is grounded in cmux source, specs, or the official sidebar plugin — not resemblance.

## D1. Target cmux-tui, not the macOS app

**Hypothesis:** The requested install UX (`cmux sidebar plugin install/use`) is the cmux-tui plugin manager.

**Evidence:** `cmux-tui/spec/plugins.md`, `plugin_manager.rs`, `cmux-sidebar-fzf` README. macOS `sidebar-state` lives in `docs/cli-contract.md` and has no plugin PTY.

**Counterargument:** The status names `needs-attention` / `review` come from the macOS workspace glyph.

**Decision:** Ship a cmux-tui sidebar plugin. Reimplement macOS lane inference on tui signals. Do not talk to the macOS socket.

## D2. Speak `cmux.protocol/2`, not crates.io `cmux-client` 0.1

**Hypothesis:** Agent state and notifications need protocol/2 `session.snapshot`.

**Evidence:** `cmux-client` 0.1 is protocol-v12 (`list_workspaces` numeric ids). `AgentSnapshot` and unread notifications are v2 catalog types. crates.io `cmux-sdk` is a name reservation. The monorepo is not a root Cargo workspace, so a git dep cannot pull `cmux-sdk` cleanly.

**Counterargument:** The official fzf plugin uses `cmux-client` 0.1; matching it would maximize install compatibility.

**Decision:** Implement a small v2 client for `session.snapshot`, `workspace.focus`, `tab.focus`. Keep the envelope identical to `bindings/rust`. If an older mux only speaks v12, show reconnect with an explicit message rather than silently degrading.

## D3. Poll `session.snapshot` at 1s instead of `session.events`

**Hypothesis:** Event-driven updates are better for CPU.

**Evidence:** `session.events` exists and carries typed upserts. Official `rust-agent-dashboard` still polls snapshot every 1s. Raw `agent-state-changed` is proposed, not implemented. Streams use a dedicated connection so they cannot share the TUI thread without extra machinery.

**Counterargument:** A second connection + event apply would cut idle CPU further.

**Decision:** MVP polls snapshot at 1s (same as the official dashboard) and keys at 100ms (same as fzf). Document events as a follow-up. Do not spawn `cmux` CLI.

## D4. Map `blocked` + unread notifications to NeedsAttention

**Hypothesis:** tui `blocked` is the needs-input signal.

**Evidence:** `agent_state_for_hook_kind` maps approval/question/plan_review/error → `Blocked`. Turn completion posts an info "finished" notification and sets `Idle`. macOS uses `anyAgentNeedsInput` first.

**Counterargument:** Treating every unread info notification as attention might be noisy.

**Decision:** Unread notification OR `blocked` → NeedsAttention. Hook install skips session-start notifications, so the noisy case is limited. Users who mark notifications read drop out of the lane (ack is out of MVP scope).

## D5. Optional `gh`, never required

**Hypothesis:** `gh` is enough for PR/CI/review without a GitHub API token in the plugin.

**Evidence:** Request asked to prefer `gh`. cmux-tui has no PR resource. macOS PR sidebar is app-private.

**Counterargument:** `gh` is another spawn; GraphQL would be richer.

**Decision:** Cache `gh pr view --json` per cwd for 20s. Missing binary, auth, network, or repo → `GitHubSignals::unavailable`. Resolver still classifies from cmux data.

## D6. Title-based agent kind, abstracted

**Hypothesis:** Provider name is not on `AgentSnapshot`.

**Evidence:** Public projection fields are listed in `RegistryAgentProjection::into_public_snapshot`. Adapter id is only in journal payload. cmux-tui labels tabs with `agent_in_title` against `["claude","codex","opencode","pi"]`.

**Counterargument:** Process argv via `terminal.process.get` might be more accurate.

**Decision:** `AgentDetector` trait with `TitleAgentDetector` (same word split as cmux-tui). Do not poll process trees in MVP. Upstream: add `adapter_id` to the snapshot.

## D7. One row per workspace

**Hypothesis:** Humans triage workspaces, not panes.

**Evidence:** macOS sidebar and the requested mock are workspace-named. Multiple agents in one workspace should collapse to the highest-priority lane.

**Counterargument:** Two blocked agents in one workspace hide the second.

**Decision:** Aggregate per workspace. Subtitle can mention extra agent count. Jump focuses the workspace and the tab of the highest-priority agent.

## D8. MIT license, no cmux fork

**Hypothesis:** Plugin OSS should match `cmux-sidebar-fzf` (MIT), not GPL cmux.

**Evidence:** fzf LICENSE is MIT. cmux-tui workspace license is MIT. cmux app is GPL-3.0-or-later.

**Decision:** MIT. Reimplement the public protocol from specs. Do not copy GPL Swift/Rust sources.

## D9. Keys: fzf first, then the requested map

**Hypothesis:** Plugin keys must not fight cmux.

**Evidence:** `spec/plugins.md`: Esc is not an exit. fzf uses Ctrl-C to quit, Up/Down/Ctrl-p/n/k/j, Enter to activate. Built-in sidebar uses Space to collapse.

**Decision:** Keep fzf navigation + Ctrl-C. Add Tab (next group), Space (collapse), r (refresh), q (quit). Esc is ignored.
