# Changelog

All notable changes to this project are documented in this file.

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

