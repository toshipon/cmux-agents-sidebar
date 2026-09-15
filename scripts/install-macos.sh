#!/usr/bin/env bash
# Install the macOS cmux custom sidebar (not cmux-tui `sidebar plugin`).
set -euo pipefail

dest="${HOME}/.config/cmux/sidebars"
src_dir="$(cd "$(dirname "$0")/.." && pwd)"
src="${src_dir}/sidebars/agents.js"

if [[ ! -f "$src" ]]; then
  echo "missing ${src}" >&2
  exit 1
fi

mkdir -p "$dest"
cp "$src" "${dest}/agents.js"
echo "Installed ${dest}/agents.js"
echo
echo "Next:"
echo "  cmux sidebar validate agents"
echo "  cmux sidebar select agents"
echo
echo "Or open as a pane / right panel:"
echo "  cmux sidebar open agents"
echo "  cmux right-sidebar set custom agents"
echo
echo "If you see 'Unknown sidebar command plugin', that is expected:"
echo "the macOS cmux CLI has no plugin subcommand. This file is the install path."
echo
echo "Back to the built-in sidebar: right-click the sidebar toggle and"
echo "choose Default Workspaces. Closing a pane opened with"
echo "'cmux sidebar open agents' is enough for that pane."
