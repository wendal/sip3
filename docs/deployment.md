# SIP3 Deployment Guide

## Prerequisites

- Docker Engine 24+ and Docker Compose v2
- MySQL 8.0 (if deploying without Docker)
- Rust 1.95+ (for building from source; required for Rust 2024 edition)
- Node.js 20+ (for frontend from source)

## Quick Start with Docker Compose

```bash
# Clone the repository
git clone https://github.com/wendal/sip3.git
cd sip3

# Start all services
docker compose up -d

# View logs
docker compose logs -f

# Check health
curl http://localhost:3000/api/health
```

Services started:
- MySQL on port 3306
- SIP server on UDP port 5060
- RTP media relay on UDP ports 10000–10099
- Conference RTP on UDP ports 10100–10199 (for audio conference rooms)
- Voicemail RTP on UDP ports 10200–10299; WAV storage mounted at `./voicemail` in Docker Compose
- REST API on port 3000
- Admin UI on port 8030

## Manual Deployment

### 1. Database Setup

```sql
CREATE DATABASE sip3;
-- Using root or create a dedicated user:
-- CREATE USER 'sip3'@'%' IDENTIFIED BY 'sip3pass';
-- GRANT ALL PRIVILEGES ON sip3.* TO 'sip3'@'%';
-- FLUSH PRIVILEGES;
```

Then run migrations:
```bash
mysql -u root -proot sip3 < migrations/001_initial.sql
mysql -u root -proot sip3 < migrations/002_seed.sql
mysql -u root -proot sip3 < migrations/003_media_sessions.sql
mysql -u root -proot sip3 < migrations/004_admin_users.sql
mysql -u root -proot sip3 < migrations/005_call_indexes.sql
mysql -u root -proot sip3 < migrations/006_acl.sql
mysql -u root -proot sip3 < migrations/007_presence.sql
mysql -u root -proot sip3 < migrations/008_numeric_seed_accounts.sql
mysql -u root -proot sip3 < migrations/009_security_events.sql
mysql -u root -proot sip3 < migrations/010_sip_messages.sql
mysql -u root -proot sip3 < migrations/011_conference_rooms.sql
mysql -u root -proot sip3 < migrations/012_voicemail.sql
```

### 2. Backend Configuration

Create `backend/config.toml`:
```toml
[server]
sip_host = "0.0.0.0"
sip_port = 5060
sip_domain = "sip.air32.cn"
api_host = "0.0.0.0"
api_port = 3000
# Public IPv4 of this server written into rewritten SDP c= lines
public_ip = "154.8.159.79"
# UDP port range for RTP media relay
rtp_port_min = 10000
rtp_port_max = 10099
# UDP port range for conference mixing
conference_rtp_port_min = 10100
conference_rtp_port_max = 10199
# Voicemail access, storage, prompts, and media range
voicemail_access_extension = "*97"
voicemail_no_answer_secs = 25
voicemail_max_message_secs = 120
voicemail_idle_timeout_secs = 10
voicemail_storage_dir = "voicemail"
voicemail_prompt_dir = "voicemail/prompts"
voicemail_rtp_port_min = 10200
voicemail_rtp_port_max = 10299

[database]
url = "mysql://root:root@localhost:3306/sip3"
max_connections = 10

[auth]
realm = "sip.air32.cn"
registration_expires = 3600
```

Or set environment variables (prefix `SIP3__`):
```bash
export SIP3__SERVER__SIP_DOMAIN=sip.air32.cn
export SIP3__SERVER__PUBLIC_IP=154.8.159.79
export SIP3__SERVER__VOICEMAIL_ACCESS_EXTENSION='*97'
export SIP3__SERVER__VOICEMAIL_NO_ANSWER_SECS=25
export SIP3__SERVER__VOICEMAIL_STORAGE_DIR=voicemail
export SIP3__SERVER__VOICEMAIL_PROMPT_DIR=voicemail/prompts
export SIP3__SERVER__VOICEMAIL_RTP_PORT_MIN=10200
export SIP3__SERVER__VOICEMAIL_RTP_PORT_MAX=10299
export SIP3__DATABASE__URL=mysql://root:root@localhost:3306/sip3
```

### 3. Build and Run Backend

```bash
cd backend
cargo build --release
./target/release/sip3-backend
```

### 4. Build and Serve Frontend

```bash
cd frontend
npm ci
npm run build
# Serve dist/ with any static file server, e.g.:
npx serve dist -p 80
```

## RTP Media Relay

SIP3 includes a built-in server-side RTP relay so clients **do not need STUN or TURN**.
When an INVITE is proxied, the server:

1. Allocates a pair of UDP ports for each active `m=audio` and `m=video` SDP section from the configured `rtp_port_min`–`rtp_port_max` range.
2. Rewrites the SDP `c=`, `m=audio`, and `m=video` fields in the INVITE to point to the server.
3. When the 200 OK arrives from the callee, rewrites its SDP the same way.
4. Learns each peer's real public RTP address from the first packet received
   (symmetric RTP – no SDP address trust required).
5. Bidirectionally forwards RTP/SRTP bytes between the two peers without transcoding.

This means clients behind NAT with no fixed public IP can call each other normally. A pure audio call consumes two relay ports; an audio+video SIP phone call consumes four. Keep Docker port ranges modest for startup performance, but size `rtp_port_min`–`rtp_port_max` for the expected concurrent video calls. Browser WebRTC video is separate from this legacy SIP RTP relay path.

## Conference Rooms

SIP3 hosts audio conference rooms as a local B2BUA endpoint with server-side mixing.

- Dial format: `sip:<9-digit-extension>@<sip-domain>` — e.g. `sip:900000000@sip.air32.cn` (default seeded room "Default Conference").
- Room extensions are 9-digit numeric, separate from the 3–6 digit user-account range to avoid collisions.
- Codecs: G.711 PCMU/PCMA only (RTP/AVP). MVP rejects SRTP/SAVP, video, and WebRTC offers.
- DTMF mute: `*6` toggles the caller's mute state. Both RFC 2833 telephone-event RTP and Linphone-style SIP `INFO application/dtmf-relay` are accepted.
- Each participant uses one UDP port from `conference_rtp_port_min`–`conference_rtp_port_max` (defaults `10100`–`10199`). Sized for ~100 concurrent participants per host. Expand both the config range and Docker port mapping if needed.
- Authentication: caller must be an existing enabled `sip_accounts` row in the local realm. No PIN in MVP.
- Manage rooms in the admin UI (sidebar → "会议室"). API: `GET/POST /api/conferences`, `PUT/DELETE /api/conferences/:id`, `GET /api/conferences/:id/participants`.


## Voicemail

SIP3 provides local voicemail mailboxes for SIP phone users.

- Delivery: calls to enabled mailboxes are answered immediately when the user is offline. For registered users who do not answer, voicemail answers after `voicemail_no_answer_secs` (default 25 seconds).
- Access: mailbox owners dial `*97` from their own extension to reach the mailbox endpoint and greeting/ready prompt. Full message playback and mailbox navigation are future work.
- Codecs: G.711 PCMU/PCMA over RTP/AVP only. The voicemail MVP excludes SRTP/SAVP, Opus, video, browser/WebRTC voicemail, full playback IVR/navigation, mailbox PINs, busy-to-voicemail routing, and email notifications.
- MWI: phones subscribe with `SUBSCRIBE Event: message-summary`; SIP3 sends `NOTIFY` (`application/simple-message-summary`) as new/saved counts change.
- Storage: messages are local WAV files under `voicemail_storage_dir`; prompts are WAV files under `voicemail_prompt_dir`. Docker Compose sets these to `/app/voicemail` and `/app/voicemail/prompts` and mounts host `./voicemail`.
- RTP: voicemail media uses `voicemail_rtp_port_min`–`voicemail_rtp_port_max` (defaults `10200`–`10299`). Open and map this UDP range independently from relay RTP (`10000`–`10099`) and conference RTP (`10100`–`10199`).
- DTMF controls: current MVP DTMF support is limited to `#` stopping an active recording. Playback controls (`1` replay, `2`/`#` next, `7` delete, `9` save, `*` exit/back) are planned but not implemented yet. RFC 2833 telephone-event RTP and SIP `INFO application/dtmf-relay` are recognized for the implemented recording stop control.
- Admin UI/API: manage boxes and messages in the admin UI. API routes are `GET/POST /api/voicemail/boxes`, `PUT /api/voicemail/boxes/:id`, `GET /api/voicemail/messages`, `PUT/DELETE /api/voicemail/messages/:id`, and `GET /api/voicemail/messages/:id/download`.

## SIP Client Configuration

Configure your SIP client (Linphone, Zoiper, etc.):

| Field       | Value                    |
|-------------|--------------------------|
| SIP Server  | sip.air32.cn             |
| Port        | 5060                     |
| Protocol    | UDP                      |
| Domain      | sip.air32.cn             |
| Username    | 1001 (or other extension) |
| Password    | password123 (from seed)  |
| Auth Method | Digest MD5               |

## Managing Accounts via API

### List accounts
```bash
curl http://localhost:3000/api/accounts
```

### Create account
```bash
curl -X POST http://localhost:3000/api/accounts \
  -H 'Content-Type: application/json' \
  -d '{"username":"dave","password":"secret","display_name":"Dave","domain":"sip.air32.cn"}'
```

### Update account
```bash
curl -X PUT http://localhost:3000/api/accounts/1 \
  -H 'Content-Type: application/json' \
  -d '{"enabled":0}'
```

### Delete account
```bash
curl -X DELETE http://localhost:3000/api/accounts/1
```

### List active registrations
```bash
curl http://localhost:3000/api/registrations
```

### List call records
```bash
curl http://localhost:3000/api/calls
```

## Firewall Rules

Open these ports:
```
UDP 5060         - SIP signaling
UDP 10000-10099  - RTP media relay
UDP 10100-10199  - Conference room RTP (G.711 mixer)
UDP 10200-10299  - Voicemail RTP (G.711 PCMU/PCMA)
UDP 20000-20099  - WebRTC ICE media
UDP/TCP 3478     - TURN/STUN
TCP 5349         - TURN/TLS
TCP 3000         - REST API (internal only in production)
TCP 8030/443     - Admin UI
TCP 3306         - MySQL (internal only)
```

## Release and Production Update

The current production layout is expected to be `root@sip.air32.cn:/opt/sip3`.

```bash
ssh root@sip.air32.cn
cd /opt/sip3
git fetch --tags origin
git checkout main
git pull --ff-only origin main
git describe --tags --always
docker compose up -d --build
docker compose ps
curl -f http://127.0.0.1:3000/api/health
```

For `v1.3.0`, verify that Docker publishes all four media ranges:

- RTP relay: `10000-10099/udp`
- Conference RTP: `10100-10199/udp`
- Voicemail RTP: `10200-10299/udp`
- WebRTC ICE: `20000-20099/udp`

Do not run destructive git commands in production unless local configuration has been backed up. Production-specific secrets should stay outside tracked files.

## Production Hardening

1. **TLS/SRTP**: Use a SIP proxy (Kamailio/OpenSIPS) with TLS in front for encrypted signaling
2. **Firewall**: Restrict SIP port to trusted IP ranges
3. **Rate limiting**: Add nginx rate limiting for the API
4. **Database**: Use strong passwords and bind MySQL to 127.0.0.1
5. **Secrets**: Use Docker secrets or a vault for passwords
6. **Logging**: Configure log aggregation (ELK, Loki, etc.)
7. **public_ip**: Set `server.public_ip` to the server's actual public IPv4 address
8. **Built-in bruteforce protection**: tune `SIP3__SECURITY__*` thresholds and keep `/api/security/*` endpoints behind JWT/API-key auth
9. **Illegal SIP INVITE scans**: if the server only accepts calls from registered endpoints, tune `SIP3__SECURITY__SIP_INVITE_IP_FAIL_THRESHOLD` and `SIP3__SECURITY__SIP_INVITE_USER_IP_FAIL_THRESHOLD` so repeated unknown-caller INVITEs are auto-banned

## Troubleshooting

### SIP registration fails
- Verify the SIP domain matches `auth.realm` in config (`sip.air32.cn`)
- Check account exists: `curl http://localhost:3000/api/accounts`
- Ensure UDP 5060 is reachable from the client

### Repeated scan / bruteforce attempts
- Check current auto-bans: `curl -H "Authorization: Bearer <JWT>" http://localhost:3000/api/security/blocks`
- Check recent security events: `curl -H "Authorization: Bearer <JWT>" "http://localhost:3000/api/security/events?limit=100"`
- Check recent illegal INVITE events only: `curl -H "Authorization: Bearer <JWT>" "http://localhost:3000/api/security/events?surface=sip_invite&event_type=invite_rejected&limit=100"`
- Check 24h summary counters (now includes INVITE abuse): `curl -H "Authorization: Bearer <JWT>" http://localhost:3000/api/security/summary`
- Check runtime troubleshooting snapshot: `curl -H "Authorization: Bearer <JWT>" http://localhost:3000/api/security/runtime`
- Manually unblock one CIDR if needed:
  `curl -X POST -H "Authorization: Bearer <JWT>" -H "Content-Type: application/json" -d '{"cidr":"203.0.113.10/32"}' http://localhost:3000/api/security/blocks/unblock`

### No audio after call connects
- Verify UDP ports 10000–10099 are open and forwarded to the server
- Verify `server.public_ip` is the actual public IP address visible to clients
- Check server logs for "Allocated media relay" and "RTP relay" messages

### Voicemail does not answer or has no audio
- Confirm the destination account has an enabled voicemail box
- For no-answer delivery, wait at least `voicemail_no_answer_secs` (default 25 seconds) before expecting voicemail to answer
- Verify UDP ports 10200–10299 are open and forwarded to the server
- Verify prompt WAV files exist under `voicemail_prompt_dir` and message WAV files can be written under `voicemail_storage_dir`
- Confirm the phone offers PCMU or PCMA over RTP/AVP; SRTP/SAVP, Opus, and WebRTC voicemail are not supported
- For MWI, confirm the phone sends `SUBSCRIBE Event: message-summary`
- If MWI or `*97` access is rejected, confirm the SIP phone is registered from the same public source IP/port used by the SUBSCRIBE/INVITE request.

### No video in Linphone after call connects
- Verify UDP ports 10000–10099 are open and forwarded to the server
- Confirm the INVITE offer and 200 OK answer both rewrite `m=video` to `server.public_ip` and a SIP3 relay port
- Remember that one audio+video SIP call uses four RTP relay ports, so port exhaustion appears sooner than with audio-only calls
- Browser WebRTC video is not handled by the legacy SIP RTP relay; it needs separate WebRTC gateway support

### Database connection fails
- Check `DATABASE_URL` is correct
- Verify MySQL is running: `systemctl status mysql`
- Check credentials: `mysql -u root -proot sip3`

### Build fails
- Rust: ensure stable toolchain is at least 1.95 (`rustup update stable`)
- Frontend: clear cache `rm -rf frontend/node_modules && npm ci`
