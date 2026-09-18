# Design decisions

Each decision is grounded in cmux source, specs, or the official sidebar plugin — not resemblance.

## D1. Ship both surfaces: cmux-tui plugin and macOS custom sidebar

**Hypothesis:** The requested install UX (`cmux sidebar plugin install/use`) is the cmux-tui plugin manager.

**Evidence:** `cmux-tui/spec/plugins.md`, `plugin_manager.rs`, `cmux-sidebar-fzf` README. macOS `CLI/cmux.swift` implements `cmux sidebar <validate|reload|select|open>` only; unknown verbs print `Unknown sidebar command '%@'`. Custom sidebars live in `~/.config/cmux/sidebars/<name>.js`.

**Counterargument:** The original request named the tui plugin commands. macOS already ships `agents-board`.

**Decision:** Keep the Ratatui plugin for cmux-tui. Also ship `sidebars/agents.js` for the macOS app, using the same StateResolver lanes against `w.agents` / `w.unread` / `w.pr`. Do not spawn processes from the JS sidebar (the runtime forbids it).

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

## D11. Show approve + mergeable from `mergeStateStatus`, not `mergeable`

**Hypothesis:** A reviewer looking at WAITING needs to see whether the PR is already approved and the merge button is live, without opening GitHub.

**Evidence:** `gh pr view --json` exposes `reviewDecision` and `mergeStateStatus`. `mergeable` is conflict-only (`MERGEABLE` even when branch protection blocks merge). macOS custom JS cannot spawn `gh` or `fetch`; current `w.pr` is `{ number, label, url, status, stale, branch }`.

**Counterargument:** Poll GitHub from the JS sidebar, or ship an ExtensionKit binary.

**Decision:** cmux-tui reads `reviewDecision` + `mergeStateStatus` (20s cache unchanged). `APPROVED` + `CLEAN` → `Approved · Ready to merge`; other approved states append `Conflict` / `Behind` / `Blocked` / `Checks failing`. macOS `agents.js` uses the same copy when those keys exist and otherwise falls back to `w.pr.label`. Upstream: project the fields onto `w.pr`. Hypothesis is unverified product-wise; this change is the cheapest way to show the signal on the surface we control.

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

## D10. macOS install is copy + `cmux sidebar select`, not `plugin`

**Hypothesis:** Users who type `cmux` on a Mac are talking to the app CLI.

**Evidence:** `Error: Unknown sidebar command 'plugin'` is the macOS CLI error for any verb other than validate/reload/select/open.

**Decision:** Document that failure as the product mismatch. Provide `scripts/install-macos.sh` and README steps that copy `sidebars/agents.js` then `cmux sidebar select agents`.

## D12. Name the open-PR idle lane Waiting, not In Review

**Hypothesis:** An operator scanning the sidebar treats **IN REVIEW** as “I should review this now.” For an idle agent with an open PR, the next actor is a reviewer (or merge), so the lane should read **WAITING**.

**Evidence:** Classification was already `open PR && agent not working`. Subtitles already say `Review pending` / `Approved · Ready to merge`. The user asked to surface that parked state as `waiting`. macOS built-in still uses `review`; this plugin is the control-plane view.

**Counterargument:** Keep **IN REVIEW** and only change the subtitle. Approved-and-mergeable PRs are not waiting on others.

**Decision:** Rename `AgentLane::InReview` → `Waiting` on both the Ratatui plugin and `sidebars/agents.js`. Same resolver rule and same detail strings. Approved PRs stay in WAITING with `Approved · …` until a later split (e.g. promote ready-to-merge to Needs Attention) is verified.
