#!/bin/sh
# Minimal OpenUU server health check: listening ports + account API probe.
# Usage: healthcheck.sh [HOST]   (default 127.0.0.1; use the public IP to test from outside)
# Exit 0 when everything passes, 1 otherwise. Safe to run from cron/systemd timers.
set -u
HOST="${1:-127.0.0.1}"
API="${OPENUU_API:-http://$HOST:21114}"
fail=0
say() { printf '%s\n' "$*"; }
bad() { say "FAIL $*"; fail=1; }

for port in 21114:account-api 21115:hbbs-nat-test 21116:hbbs-id 21117:hbbr-relay; do
  p=${port%%:*}; name=${port#*:}
  if command -v nc >/dev/null 2>&1; then
    nc -z -w 3 "$HOST" "$p" >/dev/null 2>&1 && say "ok   tcp/$p $name" || bad "tcp/$p $name not reachable"
  else
    (exec 3<>"/dev/tcp/$HOST/$p") 2>/dev/null && say "ok   tcp/$p $name" || bad "tcp/$p $name not reachable"
  fi
done

if [ "$HOST" = "127.0.0.1" ] && command -v ss >/dev/null 2>&1; then
  ss -lun 2>/dev/null | grep -q ':21116 ' && say "ok   udp/21116 hbbs-id listening" || bad "udp/21116 hbbs-id not listening"
fi

# /api/login-options must answer "[]" (password-only login, no registration).
body=$(curl -sS --noproxy '*' -m 5 "$API/api/login-options" 2>/dev/null)
[ "$body" = "[]" ] && say "ok   $API/api/login-options -> []" || bad "$API/api/login-options -> '${body:-no response}'"

# Unauthenticated /api/currentUser must be rejected with 401.
code=$(curl -sS --noproxy '*' -m 5 -o /dev/null -w '%{http_code}' -X POST "$API/api/currentUser" 2>/dev/null)
[ "$code" = "401" ] && say "ok   $API/api/currentUser unauthenticated -> 401" || bad "$API/api/currentUser unauthenticated -> '${code:-none}' (expected 401)"

if [ "$HOST" = "127.0.0.1" ] && command -v systemctl >/dev/null 2>&1; then
  for unit in openuu-account openuu-hbbs openuu-hbbr; do
    systemctl is-active --quiet "$unit" && say "ok   $unit active" || bad "$unit not active"
  done
fi

[ "$fail" -eq 0 ] && say "HEALTHY" || say "UNHEALTHY"
exit "$fail"
