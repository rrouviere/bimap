#!/usr/bin/env bash
# Launches a tmux session with three panes attached to the bimap clab lab:
#
#   ┌───────────────────────────┬───────────────────────────┐
#   │  server  (top-left)        │  client  (top-right)       │
#   │  bimap server ...          │  bimap client ...          │
#   ├───────────────────────────┴───────────────────────────┤
#   │  firewall  (bottom, full width)                         │
#   │  nftables rule flips                                     │
#   └─────────────────────────────────────────────────────────┘
#
# Each pane's first demo command is pre-typed (no Enter pressed) so the
# operator can review, paste the real fingerprint over the placeholder in the
# client pane, and fire each command with a single Enter.
#
# Run from anywhere; resolves paths relative to this script.

set -euo pipefail

SESSION="${SESSION:-bimap-demo}"
LAB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$LAB_DIR/../.." && pwd)"
BIMAP="$REPO_ROOT/target/release/bimap"
TOPO="$LAB_DIR/bimap.clab.yml"

bold() { printf '\033[1m%s\033[0m\n' "$*"; }

# Sanity checks ---------------------------------------------------------------
if ! command -v containerlab >/dev/null 2>&1; then
  echo "containerlab not found. Install: https://containerlab.dev/install/" >&2
  exit 2
fi
if ! command -v tmux >/dev/null 2>&1; then
  echo "tmux not found. Install: apt install tmux" >&2
  exit 2
fi
if [[ ! -x "$BIMAP" ]]; then
  echo "Build bimap first:  cargo build --release   (expected at $BIMAP)" >&2
  exit 2
fi

# If the lab is not running yet, start it. ------------------------------------
# Detect via docker ps rather than `containerlab inspect` output, which uses
# box-drawing chars that are awkward to grep.
if ! docker ps --format '{{.Names}}' | grep -qx server >/dev/null 2>&1; then
  bold "Deploying lab…"
  containerlab deploy -t "$TOPO"
  echo
  # containerlab deploy blocks until all exec steps finish (the apt-get install
  # of iproute2 + nftables on the firewall is one of them), so by the time it
  # returns the lab is fully configured. No extra sleep needed.
fi

bold "Lab running. Attaching tmux panes…"
bold "Server:    bimap server --bind 0.0.0.0:4242"
bold "Client:    bimap client --control-server 10.0.1.2:4242 --fingerprint <paste> --test 1kb --port-range tcp/1-200 -q"
bold "Firewall:  nft list chain inet bimap forward_chain   # show current rules"
bold "(--bidir cross-host is a future demo; see README's 'Why bidir?' section)"
bold

# Build the session -----------------------------------------------------------
tmux kill-session -t "$SESSION" 2>/dev/null || true

# Layout plan: server top-left, client top-right, firewall spanning the full
# width at the bottom. Build it in three steps, capturing a stable pane_id
# (`%N`) for each so we can address them regardless of how tmux reindexes.
#
#   1. new-session  → server pane  (%0, full window)
#   2. split -v     → firewall pane below server (%1, full width bottom)
#   3. split -h     → client pane beside server (%2, top-right)
tmux new-session -d -s "$SESSION" -n bimap \
  "docker exec -it server /bin/bash"
SERVER_PANE=$(tmux list-panes -t "$SESSION:0" -F '#{pane_id}' | head -1)

FIREWALL_PANE=$(tmux split-window -v -t "$SERVER_PANE" -P -F '#{pane_id}' \
  "docker exec -it firewall /bin/bash")

CLIENT_PANE=$(tmux split-window -h -t "$SERVER_PANE" -P -F '#{pane_id}' \
  "docker exec -it client /bin/bash")

# Wait for each container's shell to render its prompt before pre-typing,
# otherwise the send-keys characters can interleave with the prompt-draw
# race inside docker exec and produce a half-prompted mess.
#
# The wait loop alone is not enough: tmux fires a SIGWINCH at every pane
# when split-window resizes it (vertical split halves rows, horizontal split
# halves columns). bash responds by redrawing its prompt on a fresh row,
# leaving a stale "root@…# " row visible above. If send-keys were to fire
# during that redraw, the typed chars land on the wrong row. The 500 ms
# "settle pause" after the loop lets every SIGWINCH-driven redraw complete
# before any keys are sent.
wait_for_prompt() {
  local pane_id=$1
  for _ in $(seq 1 50); do
    if [[ "$(tmux capture-pane -t "$pane_id" -p 2>/dev/null)" == *root@* ]]; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}
bold "Waiting for shell prompts…"
warned=0
for pane_id in "$SERVER_PANE" "$CLIENT_PANE" "$FIREWALL_PANE"; do
  if ! wait_for_prompt "$pane_id"; then
    echo "warning: a pane never showed a prompt in 5 s — typing anyway" >&2
    warned=1
    break
  fi
done
if [[ $warned -eq 0 ]]; then
  sleep 0.5  # absorb SIGWINCH-driven prompt redraws from the splits
fi

# Pre-type the demo commands WITHOUT pressing Enter. The operator reviews,
# (replaces the fingerprint placeholder in the client pane), then fires each
# command with a single Enter. tmux send-keys with no trailing Enter / C-m
# just types the string — it does not execute it.
tmux send-keys -t "$SERVER_PANE" \
  "bimap server --bind 0.0.0.0:4242"

tmux send-keys -t "$CLIENT_PANE" \
  "bimap client --control-server 10.0.1.2:4242 --test 1kb --port-range tcp/1-300"

tmux send-keys -t "$FIREWALL_PANE" \
  "nft list chain inet bimap forward_chain"

# Enable the pane-border titles only AFTER pre-typing. Setting
# pane-border-status=top earlier would fire a SIGWINCH at every docker exec
# shell as the usable height shrinks by one row, causing bash to redraw its
# prompt on a new line and interleave with the typed text.
tmux set -g pane-border-status top
tmux select-pane -t "$SERVER_PANE" -T server
tmux select-pane -t "$CLIENT_PANE" -T client
tmux select-pane -t "$FIREWALL_PANE" -T firewall

# Land the operator on the server pane ready to fire its command.
tmux select-pane -t "$SERVER_PANE"

bold "tmux session: $SESSION   (detach with Ctrl-B d; reattach: tmux attach -t $SESSION)"
bold "Each pane has its first command pre-typed — review, then press Enter to fire."

exec tmux attach-session -t "$SESSION"
