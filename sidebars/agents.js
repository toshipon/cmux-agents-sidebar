// agents: attention-first control plane for the macOS cmux app.
//
// `cmux sidebar plugin` is cmux-tui only. This file is a custom sidebar for
// the Ghostty cmux CLI (`cmux sidebar select|open`, `cmux right-sidebar set custom`).
//
// Lane rules mirror src/state.rs StateResolver, using macOS live data:
//   needs_input ≈ tui blocked, ended ≈ tui done, w.unread, w.pr.
// JS custom sidebars cannot spawn `gh`; PR/CI come from cmux's `w.pr`.
// reviewDecision / mergeStateStatus / reviewTurn are optional until cmux projects them.
//
//   mkdir -p ~/.config/cmux/sidebars
//   cp sidebars/agents.js ~/.config/cmux/sidebars/agents.js
//   cmux sidebar validate agents
//   cmux sidebar select agents
//   # or: cmux sidebar open agents
//   # or: cmux right-sidebar set custom agents

const LANES = {
  needs_attention: { id: "needs_attention", label: "NEEDS ATTENTION", color: "#FF9F0A", strong: true },
  working: { id: "working", label: "WORKING", color: "#0A84FF", strong: false },
  in_review: { id: "in_review", label: "IN REVIEW", color: "#BF5AF2", strong: false },
  done: { id: "done", label: "DONE", color: "#7f7f7f66", strong: false },
  idle: { id: "idle", label: "IDLE", color: "#34C759", strong: false },
};
const ORDER = ["needs_attention", "working", "in_review", "done", "idle"];

const epoch = () => data.clock()?.epoch ?? 0;

function fmt(secs) {
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return s + "s";
  const m = Math.floor(s / 60);
  if (m < 60) return m + "m";
  return Math.floor(m / 60) + "h";
}

function kindLabel(kind) {
  const raw = String(kind ?? "").trim();
  if (!raw) return "";
  const lower = raw.toLowerCase();
  if (lower === "claude" || lower.indexOf("claude") === 0) return "Claude";
  if (lower === "codex" || lower.indexOf("codex") === 0) return "Codex";
  if (lower.indexOf("opencode") >= 0) return "OpenCode";
  if (lower === "pi") return "Pi";
  return raw;
}

function dirTail(path) {
  const parts = String(path ?? "").split("/").filter((p) => p.length > 0);
  return parts.length ? parts[parts.length - 1] : "";
}

function groupName(w, groups) {
  const id = w.group;
  if (!id) return "";
  for (const g of groups) {
    if (g.id === id) return String(g.name ?? "").trim();
  }
  return "";
}

// Same string Default Workspaces shows: customTitle ?? live title.
// Folder / branch are extras for the subtitle, not a replacement.
function workspaceName(w) {
  const title = String(w.title ?? "").trim();
  if (title) return title;
  const fromDir = dirTail(w.directory);
  if (fromDir) return fromDir;
  const branch = String(w.branch ?? "").trim();
  if (branch) return branch;
  return "workspace";
}

function promptText(w, primary, name) {
  const prompt = String(
    (primary && (primary.title || primary.name)) || w.latestPrompt || "",
  ).trim();
  if (prompt && prompt !== name) return prompt;
  return "";
}

function agentRank(status) {
  if (status === "needs_input") return 0;
  if (status === "working") return 1;
  if (status === "idle") return 2;
  if (status === "ended") return 4;
  return 3;
}

function pickPrimary(agents) {
  let best = null;
  for (const a of agents) {
    if (!best || agentRank(a.status) < agentRank(best.status)) best = a;
  }
  return best;
}

function prField(pr, names) {
  for (let i = 0; i < names.length; i++) {
    const value = pr[names[i]];
    if (value != null && value !== "") return String(value);
  }
  return "";
}

function mergeBlocker(mergeState, mergeable) {
  const state = String(mergeState || "").toUpperCase();
  const able = String(mergeable || "").toUpperCase();
  if (state === "DIRTY" || able === "CONFLICTING") return "Conflict";
  if (state === "BEHIND") return "Behind";
  if (state === "BLOCKED") return "Blocked";
  if (state === "UNSTABLE") return "Checks failing";
  if (state === "DRAFT") return "Draft";
  return "";
}

// Keep in lockstep with src/state.rs review_detail.
// Current cmux w.pr has no review/merge/turn fields; missing keys fall back to label.
function prReviewDetail(pr) {
  if (!pr) return "";
  if (pr.status === "merged") return "Merged";
  if (pr.stale) return "PR stale";
  const turn = prReviewTurn(pr);
  if (turn === "awaiting_reply") return "Waiting for reply";
  const decision = prField(pr, ["reviewDecision", "review_decision"]).toUpperCase();
  const mergeState = prField(pr, [
    "mergeStateStatus",
    "merge_state_status",
    "mergeableState",
    "mergeable_state",
  ]).toUpperCase();
  const mergeable = prField(pr, ["mergeable"]).toUpperCase();
  if (pr.isDraft || pr.draft) return "Draft";
  if (decision === "APPROVED") {
    if (mergeState === "CLEAN" || mergeState === "HAS_HOOKS") return "Approved · Ready to merge";
    const blocker = mergeBlocker(mergeState, mergeable);
    return blocker ? "Approved · " + blocker : "Approved";
  }
  if (decision === "REVIEW_REQUIRED" || decision === "CHANGES_REQUESTED") return "Review pending";
  return pr.label || "PR open";
}

function prReviewTurn(pr) {
  if (!pr) return "";
  const explicit = prField(pr, ["reviewTurn", "review_turn"]).toLowerCase();
  if (explicit === "needs_reply" || explicit === "needs-reply") return "needs_reply";
  if (explicit === "awaiting_reply" || explicit === "awaiting-reply" || explicit === "waiting") {
    return "awaiting_reply";
  }
  return "";
}

// Same priority as StateResolver::resolve. Keep in lockstep with src/state.rs.
function resolveLane(ws, primary) {
  const unread = (ws.unread ?? 0) > 0;
  const status = primary ? primary.status : null;
  const working = status === "working";
  const blocked = status === "needs_input";
  const ended = status === "ended";
  const pr = ws.pr;
  const prOpen = !!(pr && pr.status === "open");
  const prMerged = !!(pr && pr.status === "merged");
  const turn = prReviewTurn(pr);

  if (unread || blocked || turn === "needs_reply") {
    let detail = "Needs attention";
    if (blocked) detail = "Waiting for input";
    else if (turn === "needs_reply") detail = "Review reply";
    return { lane: "needs_attention", detail: detail };
  }
  if (prOpen && !working) {
    return { lane: "in_review", detail: prReviewDetail(pr) };
  }
  if (working) {
    return { lane: "working", detail: ws.branch || "Working" };
  }
  if (prMerged) {
    return { lane: "done", detail: "Merged" };
  }
  if (ended) {
    return { lane: "done", detail: "Done" };
  }
  return { lane: "idle", detail: "Idle" };
}

function subtitle(ws, primary, extra, detail, name, folder, group) {
  const parts = [];
  if (group && group !== name) parts.push(group);
  if (folder && folder !== name && folder !== group) parts.push(folder);
  const kind = kindLabel(primary && primary.kind);
  if (kind) parts.push(kind);
  const prompt = promptText(ws, primary, name);
  if (prompt) parts.push(prompt);
  if (extra > 0) parts.push("+" + extra);
  if (ws.pr && ws.pr.number) parts.push("#" + ws.pr.number);
  if (detail && parts.indexOf(detail) < 0) parts.push(detail);
  return parts.join(" · ");
}

const tasks = computed(() => {
  const now = epoch();
  const groups = data.groups() ?? [];
  const out = [];
  for (const w of data.workspaces() ?? []) {
    const agents = w.agents ?? [];
    const pinned = !!w.pinned;
    if (!pinned && agents.length === 0 && !(w.unread > 0) && !w.pr) continue;
    const primary = pickPrimary(agents);
    const resolved = resolveLane(w, primary);
    const ageSecs = primary
      ? (primary.sinceEpoch ? now - primary.sinceEpoch
        : primary.lastActivityAt ? now - primary.lastActivityAt
        : null)
      : null;
    const extra = agents.length > 0 ? agents.length - 1 : 0;
    const name = workspaceName(w);
    const folder = dirTail(w.directory);
    out.push({
      key: w.id,
      wsId: w.id,
      title: name,
      selected: !!w.selected,
      pinned: !!w.pinned,
      surfaceId: primary && primary.surfaceId,
      lane: resolved.lane,
      age: ageSecs != null ? fmt(ageSecs) : "",
      subtitle: subtitle(w, primary, extra, resolved.detail, name, folder, groupName(w, groups)),
      prUrl: w.pr && w.pr.url,
    });
  }
  out.sort((a, b) => (a.title < b.title ? -1 : a.title > b.title ? 1 : 0));
  return out.slice(0, 80);
});

const byLane = (lane) => () => tasks().filter((t) => !t.pinned && t.lane === lane);
const pinnedTasks = () => tasks().filter((t) => t.pinned);

const [collapsed, setCollapsed] = signal({});

function toggle(lane) {
  const next = Object.assign({}, collapsed());
  next[lane] = !next[lane];
  setCollapsed(next);
}

function jump(task) {
  cmux("workspace.select", { workspace_id: task.wsId });
  if (task.surfaceId) cmux("surface.focus", { surface_id: task.surfaceId });
}

function row(item, strong, showPin) {
  const meta = () => LANES[item().lane] ?? LANES.idle;
  const title = Text(() => item().title)
    .font(13).weight("semibold")
    .lineLimit(1).truncation("tail").marquee()
    .frame({ maxWidth: "infinity", alignment: "leading" });
  const titleLine = showPin
    ? HStack({ spacing: 4 }, [
        Image("pin.fill").font(9).color("tertiary"),
        title,
      ]).frame({ maxWidth: "infinity" })
    : title;
  return HStack({ spacing: 8 }, [
    Circle({ size: 7 }).fill(() => (item().selected ? "accent" : meta().color)),
    VStack({ spacing: 1 }, [
      titleLine,
      HStack({ spacing: 6 }, [
        Text(() => item().subtitle)
          .font(10).color("tertiary")
          .lineLimit(1).truncation("tail")
          .frame({ maxWidth: "infinity", alignment: "leading" }),
        Text(() => item().age)
          .font(10).monospaced().color("tertiary"),
      ]).frame({ maxWidth: "infinity" }),
    ]).frame({ maxWidth: "infinity" }),
  ])
    .paddingHorizontal(10).paddingVertical(6)
    .cornerRadius(8)
    .background(() => (strong ? "#FF9F0A1a" : null))
    .hoverBackground(strong ? "#FF9F0A2e" : "#7f7f7f24")
    .frame({ maxWidth: "infinity" })
    .onTap(() => jump(item()))
    .contextMenu([
      Button("Jump to workspace", () => jump(item())),
      Button(() => (item().pinned ? "Unpin" : "Pin"), () =>
        cmux("workspace.action", {
          action: item().pinned ? "unpin" : "pin",
          workspace_id: item().wsId,
        })),
      Button("Open pull request", () => { if (item().prUrl) openURL(item().prUrl); }),
    ]);
}

function laneSection(lane) {
  const meta = LANES[lane];
  const items = byLane(lane);
  return VStack({ spacing: 3 }, [
    HStack({ spacing: 6 }, [
      Text(() => (collapsed()[lane] ? "▸ " : "▾ ") + meta.label)
        .font(10).weight("semibold")
        .color(() => (items().length && meta.strong ? meta.color : "tertiary")),
      Spacer(),
      Text(() => (items().length ? String(items().length) : ""))
        .font(10).monospaced().color("tertiary"),
    ])
      .paddingHorizontal(10)
      .onTap(() => toggle(lane)),
    ForEach(
      { items: () => (collapsed()[lane] ? [] : items()), key: (t) => t.key },
      (t) => row(t, meta.strong, false),
    ),
    Text(() => (!collapsed()[lane] && items().length === 0 ? "—" : ""))
      .font(10).color("tertiary").paddingHorizontal(10),
  ]);
}

function pinnedSection() {
  return VStack({ spacing: 3 }, [
    HStack({ spacing: 6 }, [
      Text(() => (pinnedTasks().length ? "PINNED" : ""))
        .font(10).weight("semibold").color("tertiary"),
      Spacer(),
      Text(() => (pinnedTasks().length ? String(pinnedTasks().length) : ""))
        .font(10).monospaced().color("tertiary"),
    ]).paddingHorizontal(10),
    ForEach(
      { items: pinnedTasks, key: (t) => t.key },
      (t) => row(t, false, true),
    ),
  ]);
}

sidebar(() =>
  VStack({ spacing: 10 }, [
    HStack({ spacing: 6 }, [
      Text("Agents").font(14).weight("semibold"),
      Spacer(),
      Text(() => String(tasks().length)).font(11).monospaced().color("tertiary"),
    ]).paddingHorizontal(10),
    pinnedSection(),
    ...ORDER.map(laneSection),
    Spacer(),
  ]).paddingHorizontal(6),
  { surface: "glass" }
)
