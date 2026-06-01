# Changelog

All notable changes to this project are documented in this file.

## Unreleased

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
