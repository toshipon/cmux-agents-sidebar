# Upstream Proposal

These would make the Agents sidebar (and any other control-plane client) cleaner. None of them block this plugin.

The plugin does not depend on these landing.

## 1. `AgentSnapshot.adapter_id`

**Today:** Hook ingress stores `payload.adapter.id` (`claude`, `codex`, `opencode`, `pi`, …). The public agent projection drops it. Sidebar plugins must guess from terminal titles.

**Proposal:** Add optional `adapter_id: string | null` to `AgentSnapshot` / `agent.list`.

**Why a plugin cannot fake this well:** Title matching fails for custom tab names and for agents whose process title is a wrapper.

## 2. Document `session.events` agent upserts for sidebar authors

**Today:** Raw subscribe `agent-state-changed` is proposed and unimplemented. Protocol/2 `session.events` already emits resource upserts, but the official dashboard still polls `session.snapshot`.

**Proposal:** A short plugin cookbook: start `session.events`, apply agent/notification/workspace deltas, resync snapshot on overflow.

## 3. Do not add GitHub to cmux-tui for this

PR/CI/review stay optional in the plugin via `gh`. Pushing GitHub into the mux would couple network credentials into the session daemon.

## 4. Project review + merge state on macOS `w.pr`

**Today:** Custom sidebars receive `{ number, label, url, status, stale, branch }`. The host already polls GitHub REST (10s focused / 60s background) but drops `mergeable` / `mergeable_state` and never asks for review decision.

**Proposal:** Keep those fields on the probe item and add them to `Workspace+CustomSidebarPullRequests.swift`:

```text
reviewDecision: APPROVED | REVIEW_REQUIRED | CHANGES_REQUESTED
mergeable: MERGEABLE | CONFLICTING | UNKNOWN
mergeStateStatus: CLEAN | BLOCKED | BEHIND | DIRTY | DRAFT | UNSTABLE | UNKNOWN
```

`sidebars/agents.js` already maps these names (and snake_case aliases) when present.

**Why a plugin cannot fake this:** The JS runtime has no network, timers, or child processes. ExtensionKit can spawn `gh`, but that is the wrong tool for a six-field projection.

## 5. Project reviewer thread turn on macOS `w.pr`

**Today:** Comment whose-turn is computed only in the cmux-tui plugin via GraphQL `reviewThreads`. Custom sidebars cannot spawn `gh`.

**Proposal:** Add `reviewTurn: awaiting_reply | needs_reply` (omit when unknown / viewer is the PR author). Optionally `viewerLogin` + last unresolved author if the host would rather let the sidebar decide.

`sidebars/agents.js` already maps `reviewTurn` / `review_turn`.
