#!/usr/bin/env bash
# Open the bimap Containerlab in three equal, vertical tmux panes.

set -euo pipefail

SESSION="${SESSION:-bimap-demo}"
LAB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$LAB_DIR/../.." && pwd)"
TOPO="$LAB_DIR/bimap.clab.yml"
BIMAP="$REPO_ROOT/target/release/bimap"

for tool in containerlab docker tmux; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "$tool not found" >&2
    exit 2
  fi
done
if [[ ! -x "$BIMAP" ]]; then
  echo "Build bimap first: cargo build --release" >&2
  exit 2
fi

mapfile -t RUNNING < <(docker ps --format '{{.Names}}')
RUNNING_NODES=()
for node in client firewall target; do
  if printf '%s\n' "${RUNNING[@]}" | grep -qx "$node"; then
    RUNNING_NODES+=("$node")
  fi
done
if [[ ${#RUNNING_NODES[@]} -eq 0 ]]; then
  containerlab deploy -t "$TOPO"
elif [[ ${#RUNNING_NODES[@]} -ne 3 ]]; then
  echo "Partial bimap lab is running (${RUNNING_NODES[*]}); stop it before launching the demo." >&2
  exit 2
fi

tmux kill-session -t "$SESSION" 2>/dev/null || true

container_shell() {
  local node=$1 role=$2 color=$3 prompt command
  prompt="\\[\\e[38;5;${color}m\\]${role}\\[\\e[0m\\] \\[\\e[38;5;244m\\]❯\\[\\e[0m\\] "
  command="PS1='$prompt'; export PS1; exec bash --noprofile --norc -i"
  printf 'docker exec -it -e TERM=xterm-256color %q bash --noprofile --norc -c %q' \
    "$node" "$command"
}

# Keep the client on the left, then make three equal-width vertical panes.
tmux new-session -d -s "$SESSION" -n discovery \
  "$(container_shell client client 81)"
CLIENT_PANE=$(tmux list-panes -t "$SESSION:0" -F '#{pane_id}' | head -1)
TARGET_PANE=$(tmux split-window -h -l 33% -t "$CLIENT_PANE" -P -F '#{pane_id}' \
  "$(container_shell target target 120)")
FIREWALL_PANE=$(tmux split-window -h -l 50% -t "$CLIENT_PANE" -P -F '#{pane_id}' \
  "$(container_shell firewall firewall 214)")

for pane in "$CLIENT_PANE" "$FIREWALL_PANE" "$TARGET_PANE"; do
  for _ in $(seq 1 50); do
    if [[ "$(tmux display-message -p -t "$pane" '#{pane_current_command}')" == bash ]]; then
      break
    fi
    sleep 0.1
  done
done

tmux set-option -t "$SESSION" pane-border-status top
tmux set-option -t "$SESSION" pane-border-style 'fg=#334155'
tmux set-option -t "$SESSION" pane-active-border-style 'fg=#2dd4bf'
tmux set-option -t "$SESSION" pane-border-format ' #[fg=#5eead4,bold]#T #[default]'
tmux set-option -t "$SESSION" status-style 'bg=#0f172a,fg=#94a3b8'
tmux set-option -t "$SESSION" status-left-length 48
tmux set-option -t "$SESSION" status-left '#[fg=#a3e635,bold] bimap #[fg=#64748b]// FIREWALL DISCOVERY'
tmux set-option -t "$SESSION" status-right '#[fg=#38bdf8]CONTAINERLAB  #[fg=#fbbf24]TCP 1–100 '
tmux select-pane -t "$CLIENT_PANE" -T client
tmux select-pane -t "$FIREWALL_PANE" -T firewall
tmux select-pane -t "$TARGET_PANE" -T target

sleep 0.5

tmux send-keys -t "$CLIENT_PANE" \
  'bimap client --control-server 10.0.1.2:4242 --test 1kb --port-range tcp/1-100 --timeout 250'
tmux send-keys -t "$FIREWALL_PANE" 'nft list chain inet bimap forward_chain'
tmux send-keys -t "$TARGET_PANE" 'bimap server --bind 0.0.0.0:4242'

tmux select-pane -t "$TARGET_PANE"
if [[ "${BIMAP_DEMO_DETACHED:-0}" == 1 ]]; then
  printf 'Prepared tmux session %s with panes: client, firewall, target\n' "$SESSION"
  exit 0
fi
cat <<EOF
Three-pane lab is ready. Start the target's bimap server, then run the scan
in the client pane. The firewall pane shows the policy and packet counters.

Attach with: tmux attach -t $SESSION
Stop with:  Ctrl-B d, then containerlab destroy -t $TOPO
EOF
exec tmux attach-session -t "$SESSION"
