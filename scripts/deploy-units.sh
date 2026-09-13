#!/bin/sh
# Install the OpenUU systemd units on a server (run on the server, needs sudo).
#
#   OPENUU_RELAY_ADDR=<public-ip>:21117 sh scripts/deploy-units.sh [--no-timer]
#
# Reads (environment or a .env file in the current directory):
#   OPENUU_RELAY_ADDR   relay address hbbs advertises to clients (required)
#   OPENUU_KEY          hbbs/hbbr -k value, default "_" (accept any key)
#   OPENUU_ACCOUNT_BIND account API bind address, default 0.0.0.0:21114
#   OPENUU_BIN_DIR      where the binaries live, default /opt/openuu-server/bin
#   OPENUU_DATA_DIR     working/data directory, default /var/lib/openuu-server
#
# Writes openuu-account/hbbs/hbbr.service (same content as systemd/*.service
# with the addresses filled in), installs systemd/healthcheck.sh next to the
# binaries, and enables openuu-healthcheck.timer unless --no-timer is given.
# Binaries are installed separately (scripts/deploy-install-bins.sh).
set -eu
[ -f .env ] && . ./.env
: "${OPENUU_RELAY_ADDR:?set OPENUU_RELAY_ADDR=<public-ip>:21117}"
OPENUU_KEY=${OPENUU_KEY:-_}
OPENUU_ACCOUNT_BIND=${OPENUU_ACCOUNT_BIND:-0.0.0.0:21114}
OPENUU_BIN_DIR=${OPENUU_BIN_DIR:-/opt/openuu-server/bin}
OPENUU_DATA_DIR=${OPENUU_DATA_DIR:-/var/lib/openuu-server}
repo=$(cd "$(dirname "$0")/.." && pwd)

id openuu >/dev/null 2>&1 || sudo useradd --system --home "$OPENUU_DATA_DIR" --shell /usr/sbin/nologin openuu
sudo install -d -o openuu -g openuu -m 700 "$OPENUU_DATA_DIR"
sudo install -d -m 755 "$OPENUU_BIN_DIR"

for service in account hbbs hbbr; do
  case "$service" in
    account) command="$OPENUU_BIN_DIR/openuu-account" ;;
    hbbs) command="$OPENUU_BIN_DIR/hbbs -r $OPENUU_RELAY_ADDR -k $OPENUU_KEY" ;;
    hbbr) command="$OPENUU_BIN_DIR/hbbr -k $OPENUU_KEY" ;;
  esac
  sudo tee "/etc/systemd/system/openuu-$service.service" >/dev/null <<UNIT
[Unit]
Description=OpenUU $service
After=network-online.target
Wants=network-online.target
[Service]
User=openuu
Group=openuu
WorkingDirectory=$OPENUU_DATA_DIR
Environment=OPENUU_ACCOUNT_DB=$OPENUU_DATA_DIR/openuu-accounts.sqlite3
Environment=OPENUU_ACCOUNT_BIND=$OPENUU_ACCOUNT_BIND
ExecStart=$command
Restart=on-failure
RestartSec=3
UMask=0077
NoNewPrivileges=true
[Install]
WantedBy=multi-user.target
UNIT
done

sudo install -m 755 "$repo/systemd/healthcheck.sh" "$OPENUU_BIN_DIR/healthcheck.sh"
sed "s#/opt/openuu-server/bin/healthcheck.sh#$OPENUU_BIN_DIR/healthcheck.sh#" "$repo/systemd/openuu-healthcheck.service" |
  sudo tee /etc/systemd/system/openuu-healthcheck.service >/dev/null
sudo install -m 644 "$repo/systemd/openuu-healthcheck.timer" /etc/systemd/system/openuu-healthcheck.timer
sudo systemctl daemon-reload
if [ "${1:-}" != "--no-timer" ]; then
  sudo systemctl enable --now openuu-healthcheck.timer
fi
echo "units installed; enable the services with scripts/deploy-install-bins.sh"
