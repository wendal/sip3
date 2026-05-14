# Copilot Instructions for SIP3

## Build, Test, and Lint

### Backend (Rust)
```bash
cd backend

cargo build                        # debug build
cargo build --release              # release build
cargo test                         # run all tests (currently 23)
cargo test test_parse_register_request  # run a single test by name
cargo clippy -- -D warnings        # lint (CI enforces zero warnings)
cargo fmt --check                  # format check (use `cargo fmt` to fix)
```

Tests live in `backend/tests/sip_test.rs` and import from the `sip3_backend` library crate directly — the backend is both a `[lib]` (`sip3_backend`) and a `[[bin]]` (`sip3-backend`).

### Frontend (Vue 3)
```bash
cd frontend

npm install        # install deps
npm run dev        # dev server on :5173, proxies /api → http://localhost:3000
npm run build      # production build → dist/
```

### Docker
```bash
docker compose up -d         # start all services
docker compose logs -f backend   # watch backend logs
```

Ports: Admin UI **:8030**, REST API **:3000**, SIP/UDP **:5060**, SIP/TLS **:5061**, SIP/WS **:5080**, SIP/WSS **:5443**, RTP relay **UDP :10000–10099**.

---

## Architecture

`main.rs` spawns two top-level services via `tokio::spawn`:
- **SIP server** (`src/sip/server.rs`) — UDP loop on `:5060`; optionally spawns TLS, WS, WSS sub-servers
- **REST API** (`src/api/mod.rs`) — Axum HTTP server on `:3000`

Both share a `MySqlPool` (SQLx) and `Config`.

### SIP request flow

```
Incoming message (any transport)
         │
  SipHandler::process_sip_msg()
         │
   method present?
  ┌──────┴──────────────┐
  No (response)          Yes (request)
  │                      │
  relay_response()       method dispatch:
  (strip proxy Via,        REGISTER  → Registrar::handle_register()
   rewrite 200 SDP,        INVITE    → Proxy::handle_invite()
   forward to caller)      ACK/BYE/CANCEL/INFO → Proxy
                           REFER/NOTIFY → Proxy
                           SUBSCRIBE → Presence::handle_subscribe()
                           OPTIONS   → 200 OK inline
```

### Transport layer

`process_sip_msg()` is transport-agnostic and returns `Result<Option<String>>`. Each transport wraps it:

| Transport | Module | How started |
|-----------|--------|-------------|
| UDP :5060 | `server.rs` | always |
| TCP+TLS :5061 | `tcp_server.rs` | when `tls_cert` + `tls_key` are set |
| WS :5080 | `ws_server.rs` | when `ws_port != 0` |
| WSS :5443 | `ws_server.rs` | when `wss_port != 0` + TLS configured |

TLS uses `native-tls` (OS cert chain). WSS reuses the same cert/key as SIP/TLS.

### RTP media relay

For each INVITE, `MediaRelay` allocates **two UDP sockets** (`relay_a`, `relay_b`) from the configured port range:
- INVITE SDP rewritten: callee receives `relay_a` address → sends RTP there → forwarded to caller
- 200 OK SDP rewritten: caller receives `relay_b` address → sends RTP there → forwarded to callee
- Peer addresses are learned from the **source of the first packet** (symmetric RTP), so private/NAT addresses in SDP are ignored.
- `a=crypto:` (SRTP/SDES) lines are **preserved unchanged** — phones perform end-to-end SRTP; the relay forwards encrypted bytes transparently. No proxy-side decryption needed.

### IP ACL

`AclChecker` is loaded from the `sip_acl` DB table (CIDR rules with priority), wrapped in `Arc<RwLock<AclChecker>>`, and refreshed every 60 s. Packets are checked **before SIP parsing** in the UDP recv loop. Default policy (`allow`/`deny`) is configurable via `acl.default_policy`.

### Presence / BLF

`Presence` handles `SUBSCRIBE` requests, persists subscriptions to `sip_presence_subscriptions`, and sends `NOTIFY` with PIDF XML. `Registrar` calls `notify_status_change()` after every register/unregister. Supports `Event: presence` and `Event: dialog`. Expired subscriptions are purged every 5 min.

### Data flow through shared state

`SipHandler` is cloned per-task and holds:
- `Registrar` (pool + config + `Presence`)
- `Proxy` (pool + config + shared socket + `PendingDialogs` + `MediaRelay` + `ActiveDialogs`)
- `PendingDialogs`: `Arc<Mutex<HashMap<call_id, caller_SocketAddr>>>` — maps in-flight INVITE call-IDs to the caller's address so responses can be relayed back
- `ActiveDialogs`: `Arc<Mutex<HashMap<call_id, DialogInfo>>>` — established dialog state for bidirectional BYE/INFO routing

---

## Key Conventions

### SIP message parsing
- All header names are **normalized to lowercase** by `normalize_header_name()`. Always access headers with lowercase names: `msg.header("via")`, `msg.header("call-id")`.
- SIP compact header forms are expanded automatically: `f`→`from`, `v`→`via`, `i`→`call-id`, `m`→`contact`, `c`→`content-type`, `l`→`content-length`.
- `SipMessage.raw` stores the **original verbatim bytes** used for proxying.

### Password storage
Accounts store **two hashes** that must stay in sync:
- `password_hash`: bcrypt (for admin API login checks)
- `ha1_hash`: `MD5(username:realm:password)` (for SIP Digest auth, computed at create/update time in `api/accounts.rs`)

Whenever a password is changed, both are recomputed atomically in a transaction.

### API handlers
Axum handlers follow the pattern:
```rust
pub async fn handler(State(state): State<AppState>, ...) -> Result<Json<Value>, (StatusCode, String)>
```
Errors are `(StatusCode, error_message_string)` tuples. Use `sqlx::Error::Database(db_err).is_unique_violation()` for duplicate-key detection — **do not match on error message strings**.

### Configuration
Config is loaded by the `config` crate with `SIP3__` prefix and `__` as the nesting separator:
- `SIP3__SERVER__SIP_PORT=5060`
- `SIP3__DATABASE__URL=mysql://...`
- `SIP3__AUTH__REALM=sip.air32.cn`
- `SIP3__SERVER__TLS_CERT=/path/to/fullchain.pem`
- `SIP3__SERVER__TLS_KEY=/path/to/privkey.pem`
- `SIP3__SERVER__WS_PORT=5080` (set to 0 to disable)
- `SIP3__SERVER__WSS_PORT=5443` (set to 0 to disable, also needs TLS cert+key)

File-based config (`backend/config.toml`) is optional and overridden by env vars.

### Database
- Raw SQLx queries (no ORM, no migration runner at startup). Migrations are plain SQL files in `migrations/` applied once via Docker entrypoint initdb.
- Account identity is `(username, domain)` — the same username can exist in multiple SIP domains.
- DB connection uses **exponential-backoff retry** (up to 10 attempts, 1s→2s→4s→…→30s cap) in `src/db.rs`.

### Docker port ranges
Keep RTP port mappings **small** (≤200 ports). Mapping thousands of UDP ports in `docker-compose.yml` causes Docker to hang on startup due to iptables rule creation overhead. Current default: `10000-10099`.

### Nonce format
SIP auth nonces are `{data}:{MAC}` (57 chars total):
- `data` = 8-char hex timestamp + 16-char hex random (24 chars)
- `MAC` = `MD5(secret:data)` (32 hex chars)

### Adding a new SIP transport
1. Create `src/sip/<transport>_server.rs` with a `run(cfg, pool, handler)` async fn
2. Add `pub mod <transport>_server` to `src/sip/mod.rs`
3. Spawn from `server.rs::run()` if the relevant config fields are set
4. The handler calls `handler.handle_tcp_msg(raw, src)` (or the UDP equivalent) — no changes to `handler.rs` needed

### Clippy pitfalls
- `tokio_tungstenite::accept_hdr_async` closure returns `Result<Response, Response>` — both arms are large structs. Add `#[allow(clippy::result_large_err)]` to the wrapping function.
- `Arc<UdpSocket>` is shared across handler clones; be careful not to introduce deadlocks by holding async mutexes across await points.
