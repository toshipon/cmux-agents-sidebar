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
