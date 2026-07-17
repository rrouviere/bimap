#!/usr/bin/env bash
# Automated bimap demo — designed for asciinema recording.
#
# Prerequisites:
#   - Docker + containerlab installed
#   - bimap built: cargo build --release
#
# Deploys a fresh lab, records the demo, then tears down.
#
# Usage:
#   asciinema rec -c ./labs/clab/demo-record.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TOPO="$SCRIPT_DIR/bimap.clab.yml"

SERVER_CONTAINER=server
CLIENT_CONTAINER=client
FIREWALL_CONTAINER=firewall
BIMAP_CLIENT="bimap client --control-server 10.0.1.2:4242 --test 1kb --port-range tcp/1-100 --timeout 200"

bold() { printf '\033[1m%s\033[0m\n' "$*"; }
section() { echo; bold "$*"; }

# ── Deploy lab ──────────────────────────────────────────────────────────
bold "Deploying containerlab lab..."
containerlab destroy -t "$TOPO" 2>&1 | tail -3 || true
containerlab deploy -t "$TOPO" 2>&1 | tail -5
sleep 2

# ── 1. Start server ─────────────────────────────────────────────────────
section "Step 1: Start bimap server"
docker exec -d "$SERVER_CONTAINER" sh -c 'bimap server --bind 0.0.0.0:4242 2>/tmp/bimap.log'
sleep 3

FINGERPRINT=$(docker exec "$SERVER_CONTAINER" cat /tmp/bimap.log \
  | grep -oP 'fingerprint: \K.*')
if [[ -z "$FINGERPRINT" ]]; then
  echo "ERROR: could not capture fingerprint" >&2
  exit 1
fi
echo "fingerprint: $FINGERPRINT"

# ── 2. Forward scan ────────────────────────────────────────────────────
section "Step 2: Forward scan — bimap reveals the firewall policy"
docker exec -t "$CLIENT_CONTAINER" \
  $BIMAP_CLIENT --fingerprint "$FINGERPRINT" || true

# ── 3. Show the firewall rules ──────────────────────────────────────────
section "Step 3: Inspect the firewall rules bimap just revealed"
docker exec "$FIREWALL_CONTAINER" nft list chain inet bimap forward_chain

# ── 4. Live policy flip — block SSH forward ─────────────────────────────
section "Step 4: Live policy flip — block SSH (tcp/22) forward"
docker exec "$FIREWALL_CONTAINER" \
  nft 'insert rule inet bimap forward_chain ip saddr 10.0.0.0/24 ip daddr 10.0.1.0/24 tcp dport 22 counter drop'

echo ""
echo "Re-scan:"
docker exec -t "$CLIENT_CONTAINER" \
  $BIMAP_CLIENT --fingerprint "$FINGERPRINT" || true

# ── 5. Restore ──────────────────────────────────────────────────────────
section "Step 5: Restore original policy"
HANDLE=$(docker exec "$FIREWALL_CONTAINER" nft -a list chain inet bimap forward_chain \
  | grep 'tcp dport 22' | grep '10.0.0.0/24' | grep -oP 'handle \K[0-9]+' | head -1)
if [[ -n "$HANDLE" ]]; then
  docker exec "$FIREWALL_CONTAINER" nft "delete rule inet bimap forward_chain handle $HANDLE"
  echo "Original policy restored."
fi

section "Tear down"
containerlab destroy -t "$TOPO" 2>&1 | tail -3
echo "Done."
