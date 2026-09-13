# Bandwidth assessment: 50 Mbps outbound on the Tencent CVM

Status: assessment only; no configuration was changed. Written 2026-09-13 for CVM ins-xxxxxxxx
(ap-nanjing, 2 vCPU / 2 GB, public IP 203.0.113.10, outbound bandwidth cap 50 Mbps).

## 1. What the cap applies to

Tencent Cloud bills and shapes **outbound** (server to Internet) traffic; inbound is not capped by the
bandwidth setting. A relay session is symmetric from the server's point of view: every byte received from
the controlled side is sent out to the controlling side and vice versa. So for `hbbr`, **outbound = the
total traffic of all relayed sessions**, and the 50 Mbps cap is the aggregate relay budget.

50 Mbps = 6.25 MB/s. In practice TCP overhead and the Tencent shaper's burst behaviour leave roughly
45 Mbps usable for payload.

`hbbs` (signaling, ID registration, hole punching) and `openuu-account` (HTTP API) are negligible:
a few KB per connection setup plus heartbeats. They never approach the cap.

## 2. Relay scenarios

### Screen streaming

OpenUU inherits RustDesk's adaptive video pipeline (VP8/VP9/AV1/H264/H265, quality tracks bandwidth).
Typical remote-desktop bitrates:

| Use | Typical bitrate | Sessions that fit in ~45 Mbps |
| --- | --- | --- |
| Office work, static desktop, 1080p | 1-3 Mbps | 15-40 |
| Active 1080p, scrolling / IDE / browsing | 4-8 Mbps | 5-10 |
| 1080p "best quality" or 2K/4K, video playback | 15-30 Mbps | 1-3 |
| Audio only | ~0.1 Mbps | — |

Because the codec is adaptive, one relayed session alone is only capped when the user picks
"best quality" at high resolution; then frame rate or quality drops to fit ~45 Mbps rather than the
session failing. Multiple concurrent relayed sessions share the cap and all degrade together.
The one direction that matters is controlled → server → controller; the reverse direction (input events)
is tiny.

### File transfer

File transfer has no adaptive component: it consumes whatever the path allows, so a single relayed
transfer saturates the link at about **5.5 MB/s**. That is 1 GB in roughly 3 minutes and 10 GB in
about 30 minutes, and while it runs any relayed screen session on the same server will visibly
degrade (the `hbbr` limiter does not prioritise video over file frames).

### hbbr's own limits versus the CVM cap

`hbbr` defaults: `SINGLE_BANDWIDTH=128` Mb/s per connection, `TOTAL_BANDWIDTH=1024` Mb/s aggregate,
downgrade to `LIMIT_SPEED=32` Mb/s after 30 minutes above 66 % of `SINGLE_BANDWIDTH`. All of these
exceed 50 Mbps, so the CVM shaper, not `hbbr`, is the effective limit today. If the CVM stays at
50 Mbps, setting `SINGLE_BANDWIDTH` to around 40 would keep one file transfer from starving other
sessions; that is a config change to decide separately.

## 3. Direct (P2P) connections are not affected

When hole punching succeeds, media and file data go directly between the two clients; the server only
carries the handshake (a few packets on UDP/TCP 21116). Their throughput is bound by the two endpoints'
own uplinks, not by the CVM. Observed from the deployment logs so far, the test sessions from the
operator's network went through the relay ("Relayrequest ... got paired"), so P2P success rate on the
real networks (home broadband, corporate NAT) should be measured before relying on it. Corporate
symmetric NAT typically forces relay.

`ALWAYS_USE_RELAY=N` is the current hbbs setting, which is correct: it lets P2P happen whenever possible.

## 4. Options if more relay capacity is needed

Prices below are the public list prices for mainland-China regions as published by Tencent Cloud and
should be confirmed in the console for ap-nanjing before purchase; they change and there are frequent
promotions.

### 4a. Raise the fixed bandwidth on this CVM (按带宽计费)

Tiered per-Mbps monthly price for a standard BGP public IP (published examples, Guangzhou):
about 20 ¥/Mbps/month for 1-2 Mbps, 25 ¥/Mbps/month for 3-5 Mbps, and about 80-90 ¥/Mbps/month for
every Mbps above 5. Pay-as-you-go hourly bandwidth follows the same tiers pro rata.

| Outbound cap | Approx. monthly cost of the bandwidth part | Relay capacity |
| --- | --- | --- |
| 5 Mbps | ~125 ¥ | one light screen session |
| 50 Mbps (current) | ~4 000 ¥ list (lower if the instance was bought on a promotion bundle) | as in section 2 |
| 100 Mbps | ~8 500 ¥ | doubles everything above |

The jump above 5 Mbps is steep by design; fixed high bandwidth is the expensive route and only makes
sense for sustained utilisation above roughly 10 %.

### 4b. Switch this IP to pay-by-traffic (按流量计费)

About 0.8 ¥/GB outbound in mainland regions, with the instance-level cap settable up to 100 Mbps
(and higher on request) at no extra fixed cost. Break-even against 50 Mbps fixed is around
5 TB/month; for a personal remote-desktop server the monthly volume is usually far lower:

| Usage pattern | Monthly outbound | Monthly cost at 0.8 ¥/GB |
| --- | --- | --- |
| 2 h/day relayed screen at 3 Mbps | ~80 GB | ~65 ¥ |
| plus 50 GB of relayed file transfer | ~130 GB | ~105 ¥ |
| heavy: 6 h/day at 8 Mbps + 200 GB files | ~850 GB | ~680 ¥ |

Switching the billing mode is done in the console on the instance's public IP settings and does not
require redeploying the server; the cap can be raised to 100 Mbps in the same dialog. Risk: a runaway
transfer or an abusive client becomes a bill instead of a slowdown, so keep the account-only relay policy
and consider the `hbbr` `TOTAL_BANDWIDTH` limiter as a cost ceiling. The current 50 Mbps package is
likely part of a bundled promotional instance price, so check whether changing the mode forfeits that.

### 4c. Split relay from signaling onto another machine

`hbbs` and `openuu-account` need very little bandwidth and must stay on a stable address that clients
are configured with. `hbbr` is the only bandwidth consumer, and clients learn its address from `hbbs`
(`-r` / `RELAY-SERVERS`). Options:

- Run `hbbr` on a cheaper high-bandwidth host (a lightweight server with a large monthly traffic
  allowance, a pay-by-traffic CVM, or a machine in a colocation/home network with good uplink) and
  point `hbbs -r` at it. All three programs must still share the account database: `hbbr` validates
  relay tickets against `OPENUU_ACCOUNT_DB`, so a remote relay needs either the SQLite file on shared
  storage (not recommended) or a small change to validate tickets over HTTP against `openuu-account`.
  That code change is the prerequisite for this option.
- Multiple relays: `-r` accepts a comma-separated list, and clients pick the first reachable one, so
  a second relay can be added as capacity grows.
- Keep the CVM as signaling/account only and drop its bandwidth to 5 Mbps once the relay has moved,
  which would remove most of the current bandwidth cost.

## 5. Recommendation

1. Measure first: run the client's built-in connection info during real sessions for a week and note
   how often "relay" appears versus "direct", and read the `hbbr` console counters (`printf 'h' | nc 127.0.0.1 21117`)
   to see actual relayed volume.
2. If relayed volume stays under a few hundred GB per month, pay-by-traffic at a 100 Mbps cap is both
   cheaper and faster than 50 Mbps fixed; this is a console change only.
3. If sustained multi-user relay use is expected, plan the relay split (4c), which needs the HTTP ticket
   validation change in `hbbr` before deployment.
4. Independently of billing, cap `SINGLE_BANDWIDTH` below the instance limit so a file transfer cannot
   starve screen sessions; leave that as a separate, reviewed configuration change.

Sources: Tencent Cloud "云服务器公网计费模式" (cloud.tencent.com/document/product/213/10578) and
"公网网络费用" (cloud.tencent.com/document/product/213/113026); RustDesk server relay limiter
documentation reproduced in [environment-variables.md](environment-variables.md).
