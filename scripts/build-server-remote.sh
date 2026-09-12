#!/bin/sh
# Build openuu-server on the Tencent CVM and (optionally) deploy it.
#
#   build-server-remote.sh            # upload the local working tree, build offline, stop
#   build-server-remote.sh --deploy   # also back up + install the binaries, restart, health-check
#
# The CVM has 1.7 GB RAM: the build is offline against ~/openuu-build/vendor
# (populated once from `cargo vendor`; Cargo.lock must match, the script checks
# this) and runs with -j 1. Run from Git Bash on the workstation; ssh alias
# `tencent-cpu` must exist. Deployment backs up /opt/openuu-server/bin to
# bin.bak-<timestamp> and rolls back automatically when the health check fails.
set -eu
HOST=${OPENUU_HOST:-tencent-cpu}
SRC=${OPENUU_SERVER_SRC:-"$HOME/openuu-server"}
STAMP=$(date +%Y%m%d-%H%M%S)
DEPLOY=0
[ "${1:-}" = "--deploy" ] && DEPLOY=1

cd "$SRC"
echo "== packing $(git log -1 --format='%h %s')"
tar czf /tmp/openuu-src-$STAMP.tgz --exclude=target --exclude=.git src libs Cargo.toml Cargo.lock build.rs
scp -q /tmp/openuu-src-$STAMP.tgz "$HOST:~/openuu-upload/openuu-src-$STAMP.tgz"
rm -f /tmp/openuu-src-$STAMP.tgz

ssh "$HOST" "set -eu
cd ~/openuu-build
old=\$(md5sum Cargo.lock | cut -d' ' -f1)
rm -rf src libs
tar xzf ~/openuu-upload/openuu-src-$STAMP.tgz
new=\$(md5sum Cargo.lock | cut -d' ' -f1)
if [ \"\$old\" != \"\$new\" ]; then echo 'Cargo.lock changed: re-run cargo vendor before building offline' >&2; exit 2; fi
. ~/.cargo/env
export DATABASE_URL=sqlite://\$HOME/openuu-build/compile-schema.sqlite3
export SODIUM_USE_PKG_CONFIG=1
echo '== building (offline, -j 1)'
cargo --config vendor.toml build --offline --locked --release --bins -j 1 2>&1 | tee ~/openuu-build-$STAMP.log | grep -E '^(error|warning: unused|\s+Compiling hbbs|\s+Finished)' || true
grep -q 'Finished .*release' ~/openuu-build-$STAMP.log
ls -la target/release/hbbs target/release/hbbr target/release/openuu-account
"

[ "$DEPLOY" = 1 ] || { echo "== built; re-run with --deploy to install"; exit 0; }

ssh "$HOST" "set -eu
bak=/opt/openuu-server/bin.bak-$STAMP
echo \"== backing up to \$bak\"
sudo cp -a /opt/openuu-server/bin \"\$bak\"
for b in hbbs hbbr openuu-account; do echo \"old \$b: \$(sudo \$bak/\$b --version 2>/dev/null | head -1 || true) \$(md5sum \$bak/\$b | cut -c1-8)\"; done
for b in hbbs hbbr openuu-account; do sudo install -m 755 ~/openuu-build/target/release/\$b /opt/openuu-server/bin/\$b; done
sudo systemctl restart openuu-account openuu-hbbs openuu-hbbr
sleep 3
if sudo systemctl is-active --quiet openuu-account openuu-hbbs openuu-hbbr && /opt/openuu-server/healthcheck.sh; then
  echo '== deployed'
  sudo journalctl -u openuu-hbbs -u openuu-hbbr -u openuu-account --since \"-1 min\" --no-pager -o cat | grep -i error || echo 'no error lines in the last minute'
else
  echo '== health check failed, rolling back' >&2
  sudo cp -a \"\$bak\"/. /opt/openuu-server/bin/
  sudo systemctl restart openuu-account openuu-hbbs openuu-hbbr
  sudo systemctl is-active openuu-account openuu-hbbs openuu-hbbr
  exit 1
fi
"
