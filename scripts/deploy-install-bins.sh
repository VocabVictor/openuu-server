#!/bin/sh
# Install freshly built binaries and (re)start the services (run on the server, needs sudo).
#
#   sh scripts/deploy-install-bins.sh [<release-dir>]     default: target/release
#
# OPENUU_BIN_DIR / OPENUU_DATA_DIR default to /opt/openuu-server/bin and
# /var/lib/openuu-server (same as deploy-units.sh). hbbr is started only after
# hbbs has written its key pair, so a first deployment gets a consistent key.
set -eu
release=${1:-target/release}
OPENUU_BIN_DIR=${OPENUU_BIN_DIR:-/opt/openuu-server/bin}
OPENUU_DATA_DIR=${OPENUU_DATA_DIR:-/var/lib/openuu-server}
for bin in openuu-account hbbs hbbr; do
  sudo install -m 755 "$release/$bin" "$OPENUU_BIN_DIR/$bin"
done
sudo systemctl enable --now openuu-account openuu-hbbs
for attempt in $(seq 1 30); do
  if sudo test -s "$OPENUU_DATA_DIR/id_ed25519.pub"; then break; fi
  sleep 1
done
sudo test -s "$OPENUU_DATA_DIR/id_ed25519.pub"
sudo systemctl enable --now openuu-hbbr
sudo systemctl restart openuu-account openuu-hbbs openuu-hbbr
sudo systemctl is-active openuu-account openuu-hbbs openuu-hbbr
