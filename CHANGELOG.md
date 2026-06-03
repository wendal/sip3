# Changelog

All notable changes to this project are documented in this file.

## [v1.9.0] - 2026-06-03

### Added
- **Prometheus /metrics endpoint** with 12 series covering registrations, calls, conferences, security, rate limiting, and background workers (`backend/src/api/metrics.rs`).
- **OpenAPI 3.1 spec + Swagger UI** at `/api/docs`; full route inventory + auth schemes (`backend/src/api/openapi.rs`).
- **Config hot reload** via `POST /api/admin/reload` and a periodic background task (`backend/src/config_watch.rs`); uses `arc_swap::ArcSwap<Config>` for lock-free reads.
- **Webhooks** with DB-backed outbox + delivery worker, HMAC-SHA256 signing (`X-Sip3-Signature`), exponential backoff retry (max 10 attempts), and admin CRUD (`backend/src/api/{webhooks,webhook_dispatcher}.rs`).
- **Voicemail email push** via SMTP (lettre + rustls); `sip_email_outbox` table drained every 15s; no-op until `email.smtp_host` is configured.
- **Custom voicemail greeting upload** (`POST /api/voicemail/boxes/:id/greeting`) accepting 8kHz mono PCM16 WAV up to 60s; user uploads override system prompts.
- **Voicemail prompt language selector** (`SIP3__SERVER__VOICEMAIL_PROMPT_LANG`, default `en`); `scripts/gen-prompts.sh` generates en + zh 8kHz WAVs via espeak-ng.
- **Conference WAV recording** foundation: per-room recorder buffer in the mixer; flush helper returns `(wav_bytes, duration_secs)`. Wire-up to `Conference::handle_invite/handle_bye` and the recordings API are tracked as follow-ups.
- **CDR CSV export** at `GET /api/calls` (or POST with `?format=csv`); UTF-8 BOM, 11 columns including the new `hangup_cause`, `sip_response_code`, `recording_key`.
- **Frontend Calls page** (`/calls`) with server-paginated table, date/status/party filters, and CSV export button.
- **`frontend/package.json` test script** wires the existing `src/utils/*.test.mjs` files into `npm test` (26 tests pass).
- **New headless_call_tester scenarios**: `invite_busy_to_vm` and `conference_pin` (skeleton registration; full flow requires DB seeding — see TODO markers).

### Changed
- `AppState.config: Config` -> `Arc<ArcSwap<Config>>`; all 30+ handler call sites now use `state.config.load().foo`. The old `Config` deep clone per request is gone.
- `migrations/` and `backend/migrations/` are back in sync; `migrations/015_voicemail_email.sql` was missing from the top-level copy.
- Voicemail boxes now return `email` + `has_greeting` in the list endpoint; the `update_box` SQL was binding `email = COALESCE(?, email)` correctly only after C5.
- Conference mix loop now also computes a room-level sum and appends it to the room recorder (when one is attached).
- `Cargo.toml` adds: `prometheus`, `utoipa`, `utoipa-swagger-ui`, `reqwest`, `sha2`, `lettre`, `arc-swap`, `csv`, `hex`, `axum multipart` feature.

### Configuration
| Parameter | Default | Description |
|-----------|---------|-------------|
| `server.voicemail_prompt_lang` | `en` | `en` or `zh`; selects `<voicemail_prompt_dir>/<lang>/<name>.wav` |
| `email.smtp_host` | empty | When set, voicemail email outbox is drained; otherwise rows accumulate |
| `email.smtp_port` | 587 | SMTP port |
| `email.smtp_username` | empty | SMTP auth username |
| `email.smtp_password` | empty | SMTP auth password |
| `email.from_address` | empty | From address (RFC 5322) |
| `email.use_tls` | false | SMTPS (TLS implicit) vs SMTP+STARTTLS |

## [v1.8.0] - 2026-06-01

### Added
- Added **Conference PIN protection**: Conference rooms now support optional PIN authentication. PIN is provided via SIP URI parameter `sip:ext;pin=XXXX`. PINs are stored as bcrypt hashes.
- Added **TURN health monitoring API**: `GET /api/turn/health` returns TURN server status including reachability check via TCP connection test.
- Added **CDR auto-archive policy**: Background task automatically purges ended CDR records older than configurable threshold (default 90 days).
- Added **API rate limiting**: In-memory sliding window rate limiter (default 1000 req/min per IP) protecting all API endpoints. Returns 429 when exceeded.
- Added **Voicemail PIN authentication**: Voicemail boxes support optional PIN protection. Users enter PIN via DTMF before accessing messages. `#` submits PIN, `*` clears.
- Added **Voicemail email field**: Database column and API support for email notification target per voicemail box.

### Configuration
| Parameter | Default | Description |
|------------|---------|-------------|
| `security.rate_limit_requests` | 1000 | Max API requests per IP per window |
| `security.rate_limit_window_secs` | 60 | Rate limiting sliding window |
| `cleanup.cdr_cleanup_interval_secs` | 86400 | CDR purge frequency (24h) |
| `cleanup.cdr_archive_days` | 90 | CDR retention period in days |
| `turn.server` | (auto) | Comma-separated TURN server URIs |

## [v1.7.0] - 2026-06-01

### Added
- Added **busy-to-voicemail routing**: When a registered callee responds to INVITE with 486/600/603 (Busy), the call is routed to voicemail if the callee has an enabled voicemail box. The original INVITE is stored and replayed to the voicemail endpoint.
- Added **voicemail IVR playback navigation**: DTMF controls during `*97` voicemail access:
  - `1` - Play previous message
  - `2` - Play next message
  - `#` - Play next message (standard voicemail behavior)
  - `7` - Delete current message (updates DB, sends MWI notification)
  - `9` - Save current message (updates DB, sends MWI notification)

### Changed
- Expanded `VoicemailMode::Playback` to track message list and current index for IVR navigation
- Added `list_messages_for_mailbox()` to load new/saved messages on voicemail access
- `VoicemailMessage` model now derives `Clone` for in-memory state management

## [v1.6.0] - 2026-06-01

### Added
- Added `sip/message.rs` module with `SipMessage` struct, `parse()`, header helpers, URI extraction (`extract_uri`, `uri_username`, `uri_host`), auth params parsing, MD5, and Via stripping.
- Added `sip/response.rs` module with `SipResponseBuilder`, `base_response`, and `finalize_response` for constructing SIP responses.
- Added `sip/errors.rs` module with custom `thiserror` types: `RegistrarError` and `ProxyError` for future error handling improvements.
- Added `CleanupConfig` to `config.rs` with 9 configurable cleanup parameters previously hardcoded as magic numbers in `server.rs`.
- Added 36 new unit/integration tests across backend and frontend.

### Changed
- Refactored `handler.rs` (~900 lines → ~557 lines) by extracting SIP message parsing to `message.rs` and response building to `response.rs`.
- Extracted 9 magic numbers from `server.rs` into `CleanupConfig` for runtime configuration.
- Updated downstream modules (voicemail, presence, conference, registrar, proxy, media, test_client) to use new import paths from `message.rs` and `response.rs`.
- Added re-exports in `handler.rs` to maintain backward compatibility during transition.

### Configuration
The following cleanup parameters are now configurable via `config.toml` or environment variables (`SIP3__CLEANUP__*`):

| Parameter | Default | Description |
|------------|---------|-------------|
| `max_concurrent_tasks` | 512 | Max concurrent UDP datagrams |
| `udp_buffer_size` | 65535 | UDP receive buffer size |
| `media_session_max_age_secs` | 7200 | Media session stale threshold (2h) |
| `media_cleanup_interval_secs` | 60 | Media cleanup frequency |
| `reg_cleanup_interval_secs` | 3600 | Registration cleanup (1h) |
| `pres_cleanup_interval_secs` | 300 | Presence cleanup (5min) |
| `acl_refresh_interval_secs` | 60 | ACL reload frequency |
| `call_cleanup_interval_secs` | 300 | Call cleanup (5min) |
| `stale_call_age_hours` | 4 | Stale call threshold |

## [v1.5.0] - 2026-05-17

### Added
- Added `/phone` browser-to-browser video calling MVP with pre-call audio/video selection, remote video rendering, and local preview.
- Added browser softphone media helper coverage for negotiated video fallback and local sender-track shutdown.
- Added GitLab CI pipeline (`.gitlab-ci.yml`) for domestic self-hosted runner builds and Harbor image publishing.

### Changed
- Updated `/phone` call teardown to stop local sender tracks on hangup, disconnect, failed setup, and video-to-audio fallback.
- Updated README and architecture docs to clarify that `/phone` video currently supports browser-to-browser calls only.
- Updated deployment documentation for dual CI topology (GitHub->GHCR and GitLab->Harbor), mutually exclusive Harbor source paths, and provenance checks.
- Updated `.env.example` Harbor deploy defaults to a Harbor-only production model with immutable tag guidance.
- Bumped backend and frontend package metadata to `1.5.0`.

### Fixed
- Fixed admin sidebar footer version display to use the frontend package version instead of hardcoded `v0.1`.
- Fixed GitLab CI `docker_publish` dind daemon connectivity configuration.
- Fixed Harbor publish safety by restricting GitLab CI image publishing to `main` and tags.

## [v1.3.0] - 2026-05-16

### Added
- Added legacy SIP phone audio+video RTP/SRTP relay support by rewriting and relaying active `m=audio` and `m=video` SDP sections.
- Added Linphone-compatible audio conference rooms with a local SIP endpoint, G.711 PCMU/PCMA mixer, `*6` mute via RFC 2833/SIP INFO, admin API/UI, and a dedicated RTP range.
- Added voicemail MVP with offline/no-answer recording, local WAV storage, MWI `message-summary` SUBSCRIBE/NOTIFY, basic `*97` mailbox access, admin API/UI, and a dedicated RTP range.
- Added project architecture documentation in `docs/architecture.md`.

### Changed
- Updated Docker, deployment, README, and AI-assistance documentation for the new conference and voicemail media ranges.
- Bumped backend and frontend package metadata to `1.3.0`.

### Fixed
- Hardened voicemail storage keys against repeated Call-ID overwrite/collision cases.
- Hardened voicemail MWI and `*97` access checks by requiring the source socket to match an active registration.
- Aligned voicemail maximum message validation with the recorder buffer cap.
- Fixed no-answer voicemail/CANCEL race handling so cancelled calls are not recorded.

## [v1.2.0] - 2026-05-15

### Added
- Added reverse WebRTC bridge flow for SIP caller -> browser callee calls.
- Added migration `backend/migrations/010_sip_messages.sql` to create `sip_messages` table reliably.
- Added regression tests for:
  - websocket callee plain-SDP reverse bridge routing
  - forwarded CANCEL target/branch consistency
  - registration source-port refresh behavior
  - media relay source-port correctness

### Changed
- Updated operational documentation (`agent.md`, `README.md`) with production troubleshooting and release notes.

### Fixed
- Fixed immediate disconnect cases caused by missing reverse bridge in SIP -> Web call direction.
- Fixed CANCEL forwarding mismatch by building target-side CANCEL with proxy branch consistency.
- Fixed MESSAGE persistence failure when database schema was incomplete.
- Fixed NAT source-port drift routing issues by refreshing sender registration source port on authenticated traffic.
- Fixed no-audio calls caused by RTP packets being sent from the wrong relay source port.
