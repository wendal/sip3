# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

SIP3 is a SIP proxy/registrar with conference and voicemail endpoints, a WebRTC browser gateway, and a Vue 3 admin UI. Backend is Rust (Tokio + Axum + SQLx/MySQL); frontend is Vue 3 + Element Plus. SIP transports: UDP :5060, TLS :5061, WS :5080, WSS :5443. Media: RTP relay UDP :10000-10099, conference :10100-10199, voicemail :10200-10299, WebRTC ICE :20000-20099.

## Common commands

Run `./scripts/local-ci.ps1` (PowerShell) from the repo root before pushing — it exercises the same `cargo fmt --check / build / test / clippy -D warnings` and `npm ci / build` steps that both CI pipelines run.

### Backend (Rust, edition 2024, requires Rust 1.95)
```bash
cd backend
cargo build                       # debug build
cargo test                        # all tests (lib + integration in tests/sip_test.rs)
cargo test test_parse_register_request   # run a single test by name
cargo clippy -- -D warnings       # CI fails on any warning
cargo fmt --check                 # use `cargo fmt` to fix
```

The backend is both a `[lib]` (`sip3_backend`) and a `[[bin]]` (`sip3-backend`). Tests in `backend/tests/` import from the library crate.

### Frontend (Vue 3, Node 20)
```bash
cd frontend
npm ci
npm run dev                       # dev server on :5173, proxies /api → :3000
npm run build                     # production build → dist/
```

### Headless SIP regression tester
```powershell
cd backend
cargo run --bin headless_call_tester -- `
  --target sip.air32.cn --tls-port 5061 --domain sip.air32.cn --realm sip.air32.cn `
  --scenario tls_basic_call `
  --caller 1001 --caller-password <pw> --callee 1003 --callee-password <pw> `
  --rtp-threshold 8 --insecure-tls
```
Scenarios: `tls_register_dual`, `tls_message_dual`, `tls_basic_call`. Non-zero exit on failure; surfaces explicit SIP failures (403/407/486/5xx) instead of generic timeouts.

### Docker / deployment
```bash
docker compose -f docker-compose.yml -f docker-compose.dev.yml up -d --build   # local dev
docker compose -f docker-compose.yml pull && docker compose up -d              # production
curl -f http://127.0.0.1:3000/api/health
```
Production for `sip.air32.cn` lives under `/opt/sip3` and pulls from `harbor.air32.cn/sip3` (see `docs/deployment.md`). Two parallel CI pipelines exist: GitHub Actions → GHCR, GitLab CI → Harbor; the Harbor `docker_publish` job is restricted to `main` and tags.

## High-level architecture

`backend/src/main.rs` spawns two long-running Tokio tasks:
- **SIP server** (`sip/server.rs`) — UDP loop on :5060; conditionally spawns TLS, WS, WSS sub-servers. ACL check happens **before** SIP parsing in the recv loop. Background tokio tasks handle periodic cleanup: expired registrations, presence subscriptions, stale active calls (ghost calls after crashes), old CDR records, ACL reload, media session age-out, WebRTC session age-out.
- **REST API** (`api/mod.rs`) — Axum HTTP server on :3000. JWT middleware (`require_jwt`) plus an alternative `X-Api-Key` middleware (`require_auth`); CORS via tower-http.

Both share a `MySqlPool` (exponential-backoff retry, 1→2→4→…→30s, 10 attempts in `db.rs`) and a `Config` loaded via the `config` crate with `SIP3__` prefix and `__` separator.

### SIP request flow

```
SipHandler::process_sip_msg() (transport-agnostic)
  │
  ├── response? → relay_response() (strip proxy Via, rewrite 200 SDP)
  └── request dispatch:
        REGISTER       → Registrar (digest auth, persist, Contact rewrite to public source)
        INVITE         → local endpoints first (conference ext / voicemail access *97 /
                          offline-or-no-answer voicemail delivery, busy-to-voicemail on
                          486/600/603 from a registered callee), else Proxy
        ACK/BYE/CANCEL/INFO → active conference/voicemail dialog if Call-ID matches, else Proxy
        REFER/NOTIFY   → Proxy (REFER triggers blind transfer)
        SUBSCRIBE      → MWI (Event: message-summary) or Presence (Event: presence / dialog)
        OPTIONS        → 200 OK inline
        MESSAGE        → Proxy + persist
```

`SipHandler` is cloned per datagram task and holds `Registrar`, `Proxy`, `Presence`, `MediaRelay`, `WebRtcGateway`, `Conference`, `Voicemail`, and three shared `Arc<Mutex<HashMap<…>>>` dialog stores: `PendingDialogs` (in-flight INVITE call-id → caller addr), `ActiveDialogs` (post-ACK dialog state for BYE/INFO routing), `PendingInvites` (original INVITE for busy-to-voicemail replay). `SecurityGuard` (per-IP / per-IP+user sliding-window fail counter with auto-ban) is shared across SIP and API paths.

### Media model

- **Generic RTP/SRTP relay** (`sip/media.rs`): two UDP sockets (`relay_a`, `relay_b`) per active SDP media section. INVITE SDP rewritten to `relay_a`, 200 OK to `relay_b`. Peer addresses learned from the source of the first packet (symmetric RTP) — private/NAT addresses in SDP are ignored. `a=crypto:` (SDES) is **preserved unchanged**; SRTP bytes are forwarded transparently. Audio-only = 2 ports, audio+video = 4 ports.
- **Conference** (`sip/conference*.rs`): 9-digit local SIP endpoint, RTP/AVP PCMU/PCMA only, one UDP port per participant from `conference_rtp_port_*`, server-side G.711 mixer, `*6` mute via RFC 2833 / SIP INFO. v1.7+ supports optional PIN via `sip:ext;pin=XXXX` (bcrypt-hashed).
- **Voicemail** (`sip/voicemail*.rs` + `storage/voicemail.rs`): local SIP endpoint for offline/no-answer recording and `*97` access. RTP/AVP PCMU/PCMA only. MWI via `SUBSCRIBE/NOTIFY Event: message-summary` (`voicemail_mwi.rs`). `*97` and MWI SUBSCRIBE require request source IP/port to match the caller's active registration source. IVR DTMF in v1.7+: `1` prev, `2`/`#` next, `7` delete, `9` save, plus optional PIN entry (`#` submit, `*` clear).
- **WebRTC gateway** (`sip/webrtc_gateway.rs`): bridges browser ICE/DTLS-SRTP ↔ plain SIP RTP. Uses the `webrtc = "0.11"` crate. TURN creds issued from `api/turn.rs` (HMAC-SHA1 time-limited) using `SIP3__TURN__*`.

### Frontend

Vue 3 + Element Plus + Pinia + Vue Router. Views in `frontend/src/views/`: `Dashboard`, `Accounts`, `Acl`, `AdminUsers`, `Conferences`, `Voicemail`, `Security`, `Status`, `Login`, `Phone` (browser softphone using `sip.js` + WebRTC). The Vite dev server proxies `/api` to `http://localhost:3000`. Admin sidebar footer reads the version from `frontend/package.json` — do not hardcode it.

## Key conventions (project-specific)

### SIP message parsing
- All header names are **lowercase** after `normalize_header_name()`. Always use lowercase: `msg.header("call-id")`, `msg.header("from")`.
- Compact forms auto-expanded: `f`→`from`, `v`→`via`, `i`→`call-id`, `m`→`contact`, `c`→`content-type`, `l`→`content-length`.
- `SipMessage.raw` holds the **original verbatim bytes** for proxying.

### Password storage — must stay in sync
Every account has two hashes recomputed atomically in `api/accounts.rs`:
- `password_hash` — bcrypt (admin API login)
- `ha1_hash` — `MD5(username:realm:password)` (SIP Digest)

### API error pattern
Axum handlers return `Result<Json<Value>, (StatusCode, String)>`. For duplicate-key detection use `sqlx::Error::Database(db_err).is_unique_violation()` — **never match on error message strings**.

### Configuration
`SIP3__` prefix + `__` nesting. Example: `SIP3__SERVER__PUBLIC_IP=154.8.159.79` (numeric IPv4 — a hostname in SDP `c=IN IP4` breaks endpoint compatibility). `backend/config.toml` is optional and overridden by env vars. `ws_port=0` and `wss_port=0` disable those transports; TLS requires both `tls_cert` and `tls_key` set.

### Database
- Raw SQLx (no ORM). `sqlx::migrate!()` embeds `backend/migrations/` at compile time; **also add new SQL files to top-level `migrations/`** for Docker init visibility.
- Account identity is `(username, domain)`. User extensions 3-6 digits; conference extensions 9 digits.
- Voicemail WAV files use UUID suffixes with create-new semantics (avoid overwrite on duplicate Call-ID).

### RTP port mapping
Keep each UDP range ≤200 ports in `docker-compose.yml`. Thousands of mappings make Docker startup hang on iptables rule creation.

### SIP nonce format
`{data}:{MAC}` (57 chars total): `data` = 8-char hex timestamp + 16-char hex random; `MAC` = `MD5(secret:data)`.

### Adding a SIP transport
1. `src/sip/<transport>_server.rs` with `async fn run(cfg, pool, handler)`
2. `pub mod <transport>_server;` in `src/sip/mod.rs`
3. Spawn from `sip/server.rs::run()` when config fields are set
4. The handler calls `handler.handle_tcp_msg(raw, src)` (or UDP equivalent) — `handler.rs` needs no changes

### Clippy pitfalls
- `tokio_tungstenite::accept_hdr_async` closure returns `Result<Response, Response>` — both arms are large; add `#[allow(clippy::result_large_err)]` to the wrapping fn.
- `Arc<UdpSocket>` is shared across handler clones; don't hold async mutexes across await points.

## Production troubleshooting playbook (from `agent.md`)

- Always trace by **Call-ID** across SIP logs, packet captures, and CDR.
- "Ringing missing / MESSAGE undelivered" → check `sip_registrations.source_ip/source_port` against the live sender socket (NAT port drift causes this).
- "MESSAGE works but INVITE doesn't ring" → check INVITE Request-URI is the registered public address, not a private Contact, and that the proxy Via carries `rport`.
- "Connected but no audio" → confirm SDP relay port matches the actual RTP source port; phones drop mismatched-source-port media.
- "Audio OK but no video" → confirm both INVITE and 200 OK SDP rewrite `m=video` to SIP3 public IP + relay port; one A/V call uses 4 relay ports, so video exhausts the port pool faster.
- "Browser WebRTC video" → not covered by the legacy SIP `m=video` relay; `webrtc_gateway.rs` is mainly audio bridging. `webrtc_gateway.rs` is the place to extend for video.
- "Conference silent" → phone must offer RTP/AVP PCMU/PCMA, UDP :10100-10199 open, participant RTP peer learned.
- "Voicemail not answering / MWI stale" → mailbox enabled, SUBSCRIBE source matches registration, UDP :10200-10299 open, storage/prompt dirs writable.
- "MESSAGE insert error 1146" → migration `010_sip_messages.sql` not applied.

## Versioning & docs

- Backend version in `backend/Cargo.toml`; frontend version in `frontend/package.json` (UI footer reads from here).
- When shipping a release, sync `CHANGELOG.md`, `README.md`, `docs/deployment.md`, `docs/architecture.md`, `.github/copilot-instructions.md`, and bump both version fields.
- Detailed architecture and component map: `docs/architecture.md`. Deployment and TLS: `docs/deployment.md`. Production incident notes: `agent.md` (Chinese).
