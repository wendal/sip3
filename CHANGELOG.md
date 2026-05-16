# Changelog

All notable changes to this project are documented in this file.

## Unreleased

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
