# Operations: logs, monitoring, account administration

Applies to the systemd deployment described in [deployment-tencent.md](deployment-tencent.md)
(units `openuu-account`, `openuu-hbbs`, `openuu-hbbr`, user `openuu`, state in `/var/lib/openuu-server`).

## Logging

All three programs log to stdout through `flexi_logger`; systemd captures stdout into the journal.
Level comes from `RUST_LOG` in the unit's process environment (default `info`). `.env` and `--config`
are loaded after logging starts, so `RUST_LOG` there has no effect.

| Level | What lands there |
| --- | --- |
| `error` | Account database unreachable, session/ticket lookups failing, listener accept failures |
| `warn` | Login failures and rate limiting, unauthorized signaling requests, relay ticket rejections, key mismatches |
| `info` | Service start, successful logins, tickets issued, relay pairings, wake requests, account administration |
| `debug` | Every SQL statement executed by the account database (was `info` before; noisy) |

Security-relevant events use a fixed `event=<name>` prefix followed by `key=value` pairs so they can be
grepped or parsed:

| Event | Program | Meaning |
| --- | --- | --- |
| `login_ok ip= user=` | openuu-account | Password accepted, session issued |
| `login_failed ip= user=` | openuu-account | Wrong password or unknown/disabled user (401) |
| `login_rate_limited ip=` | openuu-account | More than 10 login attempts from one IP within 60 s (429) |
| `login_error ip= err=` | openuu-account | Database or hashing failure during login (500) |
| `relay_ticket_issued relay=` / `relay_ticket_denied` | openuu-account | `/api/relay-ticket` result |
| `wake_queued user= target= helper=` / `wake_rejected user= target= state=` | openuu-account | Wake-on-LAN request accepted or refused |
| `account_created` / `password_changed` / `account_disabled` / `account_enabled` / `account_deleted user=` | openuu-account CLI | Operator actions |
| `punch_unauthorized from= peer=` / `relay_request_unauthorized from= peer=` | hbbs | Connection attempt without a valid account session |
| `relay_denied from= relay=` and `relay_ticket_rejected relay=` | hbbr | Relay handshake refused (missing, expired, reused or foreign ticket) |
| `account_db_unavailable err=` | hbbs / hbbr | Shared SQLite database could not be opened; every check then denies |

Passwords and tokens are never logged. Login failures log the submitted user name (truncated to 64 chars, quoted) to expose brute-force patterns.

### Reading logs with journalctl

```sh
# Live tail of all three services
sudo journalctl -u openuu-account -u openuu-hbbs -u openuu-hbbr -f -o short-iso

# Last hour, one service
sudo journalctl -u openuu-account --since -1h --no-pager

# Only security events
sudo journalctl -u openuu-account -u openuu-hbbs -u openuu-hbbr --since today | grep -E 'event=(login_failed|login_rate_limited|.*unauthorized|relay_denied)'

# Successful logins per user today
sudo journalctl -u openuu-account --since today | grep -o 'event=login_ok.*' | sort | uniq -c

# Warnings and errors only (journal priority is not set by the app; filter on text)
sudo journalctl -u openuu-hbbs --since -1d | grep -E ' (WARN|ERROR) '
```

To raise verbosity temporarily (shows SQL statements), add a drop-in and restart the unit:

```sh
sudo systemctl edit openuu-account   # add: [Service]\nEnvironment=RUST_LOG=debug
sudo systemctl restart openuu-account
```

Journal retention is controlled by `journald.conf` (`SystemMaxUse=`); on a 2 GB CVM keep it bounded.

## Health check

`systemd/healthcheck.sh` checks TCP 21114-21117, UDP 21116 (local only), `GET /api/login-options`
(must return `[]`), `POST /api/currentUser` without a token (must return 401) and, when run locally,
that the three units are active. It exits 0 on success and 1 on any failure.

```sh
# On the server
sh /opt/openuu-server/healthcheck.sh

# From an operator machine against the public address
sh systemd/healthcheck.sh 203.0.113.10
```

Run it every few minutes from cron or a systemd timer and alert on non-zero exit, for example:

```
*/5 * * * * /opt/openuu-server/healthcheck.sh >/dev/null || logger -t openuu-health "OpenUU health check failed"
```

`openuu-toolchain/verify-deployment.py` remains the end-to-end check (real logins and an authenticated relay round trip); it needs the test credentials file and is not meant for unattended use.

Other read-only inspection commands:

```sh
systemctl status openuu-account openuu-hbbs openuu-hbbr
ss -lntup | grep -E '2111[4-9]'
printf 'h' | nc 127.0.0.1 21117        # hbbr console: command list, bandwidth counters
```

## Account administration

Accounts live in the shared SQLite database (`OPENUU_ACCOUNT_DB`). Public registration stays disabled;
the operator manages accounts with the `openuu-account` binary, which must run with the same
`OPENUU_ACCOUNT_DB` and as a user that can write the database file (on the CVM: `openuu`).

```sh
cd /var/lib/openuu-server
export OPENUU_ACCOUNT_DB=/var/lib/openuu-server/openuu-accounts.sqlite3
sudo -u openuu -E env OPENUU_NEW_PASSWORD='...' /opt/openuu-server/bin/openuu-account create alice
sudo -u openuu -E env OPENUU_NEW_PASSWORD='...' /opt/openuu-server/bin/openuu-account passwd alice
sudo -u openuu -E /opt/openuu-server/bin/openuu-account disable alice
sudo -u openuu -E /opt/openuu-server/bin/openuu-account enable alice
sudo -u openuu -E /opt/openuu-server/bin/openuu-account delete alice
sudo -u openuu -E /opt/openuu-server/bin/openuu-account list
```

- `create`/`passwd` read the password from `OPENUU_NEW_PASSWORD` (12-72 bytes) and remove it from
  their own environment; prefer `read -s` into a variable over typing it on a command line that lands in shell history.
- `passwd` and `disable` revoke the account's sessions and relay tickets immediately.
- `delete` also removes the account's wake-on-LAN device registrations and queued wake jobs.
- The CLI writes to the database while the service is running; SQLite's busy timeout (5 s) handles the concurrency.
- Existing services do not need a restart after account changes.
