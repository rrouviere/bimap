#!/usr/bin/env bash
# Record a live three-pane Containerlab firewall scan as an asciicast.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TOPO="$SCRIPT_DIR/bimap.clab.yml"
CAST="${1:-$SCRIPT_DIR/demo.cast}"
SESSION="${SESSION:-bimap-record}"
BIMAP="$REPO_ROOT/target/release/bimap"
LAB_OWNED=0
SESSION_READY=0
DRIVER_PID=""

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 not found" >&2
    exit 2
  fi
}

for tool in docker containerlab tmux asciinema; do need "$tool"; done
if [[ ! -x "$BIMAP" ]]; then
  echo "Build bimap first: cargo build --release" >&2
  exit 2
fi

cleanup() {
  if [[ -n "$DRIVER_PID" ]]; then
    kill "$DRIVER_PID" 2>/dev/null || true
  fi
  if [[ "$SESSION_READY" == 1 ]]; then
    tmux kill-session -t "$SESSION" 2>/dev/null || true
  fi
  if [[ "$LAB_OWNED" == 1 ]]; then
    containerlab destroy -t "$TOPO" >/dev/null 2>&1 || true
    rm -rf "$SCRIPT_DIR/clab-bimap"
  fi
}
trap cleanup EXIT

mapfile -t RUNNING < <(docker ps --format '{{.Names}}')
if printf '%s\n' "${RUNNING[@]}" | grep -qx target \
  && printf '%s\n' "${RUNNING[@]}" | grep -qx client \
  && printf '%s\n' "${RUNNING[@]}" | grep -qx firewall; then
  echo "Reusing the running Containerlab topology."
else
  for node in target client firewall; do
    if printf '%s\n' "${RUNNING[@]}" | grep -qx "$node"; then
      echo "Partial bimap lab is running; stop it before recording (container: $node)." >&2
      exit 2
    fi
  done
  echo "Deploying the bimap Containerlab topology..."
  DEPLOY_LOG=$(mktemp)
  if ! containerlab deploy -t "$TOPO" >"$DEPLOY_LOG" 2>&1; then
    cat "$DEPLOY_LOG" >&2
    rm -f "$DEPLOY_LOG"
    exit 1
  fi
  grep -E 'Parsing & checking|Created container|Created link|^│ (client|firewall|target)' \
    "$DEPLOY_LOG" || true
  rm -f "$DEPLOY_LOG"
  LAB_OWNED=1
fi

SESSION="$SESSION" BIMAP_DEMO_DETACHED=1 "$SCRIPT_DIR/demo.tmux.sh"
SESSION_READY=1

mapfile -t PANES < <(tmux list-panes -t "$SESSION" -F '#{pane_id} #{pane_title}')
CLIENT_PANE=""
FIREWALL_PANE=""
TARGET_PANE=""
for pane in "${PANES[@]}"; do
  case "$pane" in
    *" client") CLIENT_PANE="${pane%% *}" ;;
    *" firewall") FIREWALL_PANE="${pane%% *}" ;;
    *" target") TARGET_PANE="${pane%% *}" ;;
  esac
done
if [[ -z "$CLIENT_PANE" || -z "$FIREWALL_PANE" || -z "$TARGET_PANE" ]]; then
  echo "Could not find all three tmux panes." >&2
  exit 1
fi

drive_demo() {
  local client_pane=$1 firewall_pane=$2 target_pane=$3 session=$4
  for _ in $(seq 1 300); do
    if [[ -n "$(tmux list-clients -t "$session" -F '#{client_name}' 2>/dev/null)" ]]; then
      break
    fi
    sleep 0.1
  done
  if [[ -z "$(tmux list-clients -t "$session" -F '#{client_name}' 2>/dev/null)" ]]; then
    echo "No recording terminal attached to tmux session $session." >&2
    return 1
  fi

  tmux send-keys -t "$firewall_pane" C-m
  sleep 1
  tmux send-keys -t "$target_pane" C-m
  for _ in $(seq 1 100); do
    if tmux capture-pane -t "$target_pane" -p | grep -q 'listening on 0.0.0.0:4242'; then
      break
    fi
    sleep 0.1
  done
  if ! tmux capture-pane -t "$target_pane" -p | grep -q 'listening on 0.0.0.0:4242'; then
    echo "The bimap server did not start." >&2
    return 1
  fi

  sleep 1
  tmux send-keys -t "$client_pane" C-m
  for _ in $(seq 1 600); do
    if tmux capture-pane -t "$client_pane" -p | grep -q 'passed, .*failed, 0 errors'; then
      break
    fi
    sleep 0.1
  done
  if ! tmux capture-pane -t "$client_pane" -p | grep -q 'passed, .*failed, 0 errors'; then
    echo "The bimap scan did not finish." >&2
    return 1
  fi
  local scan_output
  scan_output=$(tmux capture-pane -t "$client_pane" -p -S -)
  if ! grep -q 'PASS 1kb tcp 22,80' <<<"$scan_output" \
    || ! grep -q '2 passed, 98 failed, 0 errors' <<<"$scan_output"; then
    echo "The scan completed, but it did not reveal the expected TCP 22/80 policy." >&2
    return 1
  fi

  sleep 2
  tmux send-keys -t "$client_pane" \
    'printf "\\n  POLICY FOUND  ·  tcp/22 + tcp/80 reachable\\n"'
  tmux send-keys -t "$client_pane" C-m
  tmux send-keys -t "$firewall_pane" C-u
  tmux send-keys -t "$firewall_pane" \
    'clear; printf "  LIVE FIREWALL RULES  ·  counters after the scan\\n\\n"; nft -a list chain inet bimap forward_chain'
  tmux send-keys -t "$firewall_pane" C-m
  sleep 3
  tmux send-keys -t "$target_pane" C-c
  sleep 0.5
  tmux display-message -t "$session" 'Scan complete  ·  discovered TCP 22 and 80'
  sleep 2
  tmux kill-session -t "$session"
}

(
  if ! drive_demo "$CLIENT_PANE" "$FIREWALL_PANE" "$TARGET_PANE" "$SESSION"; then
    tmux kill-session -t "$SESSION" 2>/dev/null || true
    exit 1
  fi
) &
DRIVER_PID=$!
echo "Recording a live client | firewall | target scan to $CAST"
TERM=xterm-256color asciinema rec --overwrite --quiet --cols 240 --rows 44 \
  --title 'bimap · firewall policy discovery in Containerlab' \
  -c "TERM=xterm-256color tmux attach-session -t $SESSION; printf '\\033[2J\\033[18;78H\\033[38;2;45;212;191m╭──────────────────────────────────────────────────────────────╮\\033[19;78H│  BIMAP  /  FIREWALL POLICY DISCOVERED                        │\\033[20;78H│  TCP 22  ALLOWED    ·    TCP 80  ALLOWED                      │\\033[21;78H│  100 ports scanned  ·  98 filtered  ·  2 reachable           │\\033[22;78H│  client ───────── firewall ───────── target                    │\\033[23;78H╰──────────────────────────────────────────────────────────────╯\\033[0m'" "$CAST"
wait "$DRIVER_PID"
DRIVER_PID=""

echo "Saved asciicast: $CAST"
