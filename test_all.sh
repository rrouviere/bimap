#!/bin/bash
BIN=./target/release/bimap
PASS=0
FAIL=0

run_test() {
    local name="$1" expected="$2" sport="$3"; shift 3
    local client_args="$@"
    echo "=== $name ==="
    
    fuser -k "${sport}/tcp" 2>/dev/null || true
    sleep 0.3
    
    $BIN server --bind "127.0.0.1:$sport" &
    local spid=$!
    sleep 1
    
    if ! kill -0 $spid 2>/dev/null; then
        echo "  FAIL — server did not start"
        FAIL=$((FAIL+1))
        wait $spid 2>/dev/null || true
        return
    fi
    
    local out
    out=$($BIN client --server 127.0.0.1 --port "$sport" --timeout 3000 $client_args 2>&1) || true
    
    kill $spid 2>/dev/null || true
    wait $spid 2>/dev/null || true
    fuser -k "${sport}/tcp" 2>/dev/null || true
    sleep 0.2
    
    if echo "$out" | grep -q "$expected"; then
        echo "  PASS"
        PASS=$((PASS+1))
    else
        echo "  FAIL — expected '$expected', got:"
        echo "$out" | while read l; do echo "    $l"; done
        FAIL=$((FAIL+1))
    fi
}

# 1. open TCP
run_test "open_tcp" "PASS open tcp" 14001 "--test open --port-range tcp/30001-30002"

# 2. open UDP
run_test "open_udp" "PASS open udp" 14002 "--test open --port-range udp/30005-30005"

# 3. TLS
run_test "tls" "PASS tls tcp" 14003 "--test tls --port-range tcp/30010-30010"

# 4. 1kb TCP
run_test "1kb_tcp" "PASS 1kb tcp" 14004 "--test 1kb --port-range tcp/30015-30015"

# 5. 1kb UDP
run_test "1kb_udp" "PASS 1kb udp" 14005 "--test 1kb --port-range udp/30016-30016"

# 6. DNS UDP
run_test "dns_udp" "PASS dns udp" 14006 "--test dns --port-range udp/30020-30020"

# 7. DNS TCP (isolated)
run_test "dns_tcp" "PASS dns tcp" 14007 "--test dns --port-range tcp/30030-30030"

# 8. Multi protocol (open TCP + DNS UDP + 1kb TCP)
echo "=== multi ==="
fuser -k 14008/tcp 2>/dev/null || true
sleep 0.3
$BIN server --bind "127.0.0.1:14008" &
spid=$!
sleep 1
out=$($BIN client --server 127.0.0.1 --port 14008 --timeout 3000 --test open --test 1kb --test dns --port-range tcp/31000-31001 --port-range udp/31002-31002 2>&1) || true
kill $spid 2>/dev/null || true; wait $spid 2>/dev/null || true
if echo "$out" | grep -q "PASS open tcp" && echo "$out" | grep -q "PASS 1kb tcp" && echo "$out" | grep -q "PASS dns udp"; then
    echo "  PASS"
    PASS=$((PASS+1))
else
    echo "  FAIL, got:"
    echo "$out" | while read l; do echo "    $l"; done
    FAIL=$((FAIL+1))
fi

# 9. Bidir
echo "=== bidir ==="
fuser -k 14009/tcp 2>/dev/null || true
sleep 0.3
$BIN server --bind "127.0.0.1:14009" &
spid=$!
sleep 1
out=$($BIN client --server 127.0.0.1 --port 14009 --timeout 3000 --test open --port-range tcp/30060-30060 --bidir 2>&1) || true
kill $spid 2>/dev/null || true; wait $spid 2>/dev/null || true
if echo "$out" | grep -q "PASS.*->" && echo "$out" | grep -q "PASS.*<-"; then
    echo "  PASS"
    PASS=$((PASS+1))
else
    echo "  FAIL, got:"
    echo "$out" | while read l; do echo "    $l"; done
    FAIL=$((FAIL+1))
fi

# 10. Server stays alive across multiple clients
echo "=== server_alive ==="
fuser -k 14010/tcp 2>/dev/null || true
sleep 0.3
$BIN server --bind "127.0.0.1:14010" &
SPID=$!
sleep 1

# Client 1
out1=$($BIN client --server 127.0.0.1 --port 14010 --timeout 3000 --test open --port-range tcp/31010-31010 2>&1) || true
# Client 2 (should work on same server)
sleep 0.5
out2=$($BIN client --server 127.0.0.1 --port 14010 --timeout 3000 --test open --port-range tcp/31011-31011 2>&1) || true

kill $SPID 2>/dev/null || true; wait $SPID 2>/dev/null || true

if echo "$out1" | grep -q "PASS open tcp" && echo "$out2" | grep -q "PASS open tcp"; then
    echo "  PASS"
    PASS=$((PASS+1))
else
    echo "  FAIL, client1:" && echo "$out1" | head -3
    echo "  client2:" && echo "$out2" | head -3
    FAIL=$((FAIL+1))
fi

# 11. Fingerprint verification
echo "=== fingerprint_verify ==="
fuser -k 14011/tcp 2>/dev/null || true
$BIN server --bind "127.0.0.1:14011" > /tmp/server_out 2>&1 &
FPID=$!
sleep 1
FP=$(cat /tmp/server_out 2>/dev/null | grep -i fingerprint | head -1 | awk '{print $NF}' | tr -d '[:space:]')
if [ -n "$FP" ]; then
    out=$($BIN client --server 127.0.0.1 --port 14011 --fingerprint "$FP" --test open --port-range tcp/30070-30070 --timeout 2000 2>&1) || true
    kill $FPID 2>/dev/null || true; wait $FPID 2>/dev/null || true
    if echo "$out" | grep -qi "fingerprint"; then
        echo "  PASS"
        PASS=$((PASS+1))
    else
        echo "  FAIL — FP=$FP, got:"
        echo "$out" | while read l; do echo "    $l"; done
        FAIL=$((FAIL+1))
    fi
else
    echo "  FAIL — no fingerprint in server output"
    cat /tmp/server_out
    kill $FPID 2>/dev/null || true; wait $FPID 2>/dev/null || true
    FAIL=$((FAIL+1))
fi

# 12. JSON output
echo "=== json_output ==="
fuser -k 14012/tcp 2>/dev/null || true
sleep 0.3
$BIN server --bind "127.0.0.1:14012" &
JSON_SPID=$!
sleep 1
out=$($BIN client --server 127.0.0.1 --port 14012 --timeout 3000 --test open --port-range tcp/31020-31020 --json 2>&1) || true
kill $JSON_SPID 2>/dev/null || true; wait $JSON_SPID 2>/dev/null || true

if echo "$out" | grep -q '"status":"pass"'; then
    echo "  PASS"
    PASS=$((PASS+1))
else
    echo "  FAIL, got:"
    echo "$out" | while read l; do echo "    $l"; done
    FAIL=$((FAIL+1))
fi

# 13. Exit codes
echo "=== exit_codes ==="
set +e
$BIN client --server 127.0.0.1 --port 1 --test open --port-range tcp/1-1 --timeout 500 2>/dev/null; ec=$?
if [ $ec -eq 3 ]; then echo "  PASS (connection error -> 3)"; PASS=$((PASS+1))
else echo "  FAIL (expected 3, got $ec)"; FAIL=$((FAIL+1)); fi

$BIN client --server 127.0.0.1 --timeout 500 2>/dev/null; ec=$?
if [ $ec -eq 0 ]; then echo "  PASS (no args -> lists tests, exit 0)"; PASS=$((PASS+1))
else echo "  FAIL (expected 0, got $ec)"; FAIL=$((FAIL+1)); fi
set -e

# 14. --test alone lists tests + exit 0
echo "=== test_listing ==="
set +e
out=$($BIN client --server 127.0.0.1 2>&1)
ec=$?
set -e
if [ $ec -eq 0 ] && echo "$out" | grep -q "available tests"; then
    echo "  PASS"
    PASS=$((PASS+1))
else
    echo "  FAIL (ec=$ec), got:" && echo "$out" | head -5
    FAIL=$((FAIL+1))
fi

echo ""
echo "Results: $PASS passed, $FAIL failed"
[ $FAIL -eq 0 ]
