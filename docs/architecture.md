# SIP3 Architecture

SIP3 is a Rust SIP server with a Vue admin UI. It combines SIP registrar/proxy behavior, local media services, a REST administration API, and Docker-based deployment.

## Runtime topology

```text
SIP phones -- UDP/TLS/WS/WSS --+
Browser phone -- WSS/SIP.js ---+-- SIP server (:5060/:5061/:5080/:5443)
                               |
                               +-- Registrar + auth + presence/BLF
                               +-- Proxy + RTP/SRTP relay
                               +-- Conference local endpoint + G.711 mixer
                               +-- Voicemail local endpoint + WAV storage + MWI
                               +-- WebRTC gateway

Admin browser -- :8030 -- Frontend -- REST API (:3000) -- MySQL 8.0
```

`backend/src/main.rs` loads configuration, opens the MySQL pool, runs embedded SQLx migrations, starts the SIP server, and starts the Axum REST API. SIP transports share the same `SipHandler`, so UDP, TLS, WS, and WSS messages follow the same routing rules after transport-specific framing.

## Main components

| Component | Primary files | Responsibility |
| --- | --- | --- |
| SIP parser and handler | `backend/src/sip/message.rs`, `backend/src/sip/handler.rs` | Normalize SIP messages, dispatch requests/responses, and route local endpoints before generic proxying. |
| Registrar | `backend/src/sip/registrar.rs` | REGISTER digest auth, registration persistence, source-address tracking, and presence status changes. |
| Proxy | `backend/src/sip/proxy.rs` | User-to-user INVITE/MESSAGE/BYE/CANCEL/INFO/REFER routing, active dialog tracking, SDP rewriting, and CDR updates. |
| RTP relay | `backend/src/sip/media.rs` | Symmetric RTP/SRTP relay for SIP phone audio/video calls. Allocates two relay sockets per active SDP media section. |
| WebRTC gateway | `backend/src/sip/webrtc_gateway.rs` | Browser ICE/DTLS-SRTP to plain SIP RTP bridging for `/phone` audio/interoperability paths. Browser-to-browser video on `/phone` stays end-to-end WebRTC between browsers. |
| Conference | `backend/src/sip/conference*.rs` | Local 9-digit room endpoint, RTP/AVP G.711 negotiation, participant lifecycle, server-side mixing, and `*6` mute. |
| Voicemail | `backend/src/sip/voicemail*.rs`, `backend/src/storage/voicemail.rs` | Offline/no-answer recording, `*97` access readiness, MWI message-summary notifications, and local WAV storage. |
| REST API | `backend/src/api/*.rs` | JWT-protected admin APIs for accounts, calls, ACL, security, conferences, voicemail, and dashboard stats. |
| Frontend | `frontend/src` | Vue 3 + Element Plus admin dashboard and `/phone` browser softphone for audio plus browser-to-browser video. |

## SIP routing order

For incoming requests, `SipHandler::process_sip_msg()` parses the method and routes in this order:

1. REGISTER goes to `Registrar`.
2. INVITE checks local endpoints first: enabled conference room extension, voicemail access extension `*97`, offline/no-answer voicemail delivery, and then generic proxy routing.
3. ACK/BYE/CANCEL/INFO are routed to an active conference or voicemail dialog when the Call-ID belongs to a local endpoint; otherwise they go to the proxy.
4. SUBSCRIBE with `Event: message-summary` is handled by voicemail MWI; other presence/dialog subscriptions go to `Presence`.
5. Responses are relayed back through proxy dialog state. 200 OK SDP can be rewritten for RTP relay/WebRTC bridge paths.

Header names are normalized to lowercase and compact forms are expanded. Use `msg.header("call-id")`, `msg.header("from")`, and other lowercase names everywhere.

## Media model

SIP3 uses separate non-overlapping UDP ranges:

| Range | Purpose |
| --- | --- |
| `10000-10099/udp` | Generic SIP phone RTP/SRTP relay. Audio-only calls consume two ports; audio+video calls consume four. |
| `10100-10199/udp` | Conference participant RTP sockets. One port per participant. |
| `10200-10299/udp` | Voicemail RTP sockets for recording and prompt playback. |
| `20000-20099/udp` | WebRTC ICE media. |

The generic relay never decrypts or transcodes SRTP; it forwards bytes from the SDP-signaled relay ports. Conference and voicemail are local media endpoints and only accept RTP/AVP G.711 PCMU/PCMA in v1.3.0.

## Data and migrations

The backend uses raw SQLx queries with MySQL. Migrations are kept in two locations:

- `migrations/` for manual/Docker initialization visibility;
- `backend/migrations/` for `sqlx::migrate!()` embedding at compile time.

Always add new migration files to both directories. Account identity is `(username, domain)`. User extensions are 3-6 digit numbers; conference extensions are 9-digit numbers.

## Deployment model

Docker Compose starts MySQL, backend, frontend, and coturn. Production for `sip.air32.cn` is expected under `/opt/sip3`, pulls pinned images from `harbor.air32.cn/sip3`, and should be updated with:

```bash
docker compose pull
docker compose up -d
curl -f http://127.0.0.1:3000/api/health
```

Keep Docker UDP port ranges modest. Mapping thousands of ports can make Docker startup slow or appear hung because each mapping expands into firewall/iptables rules.

## v1.3.0 boundaries

- Conference MVP supports Linphone-compatible audio rooms only: no PIN, SRTP, video, Opus, or browser/WebRTC conference participation.
- Voicemail MVP supports recording, storage, MWI, admin management, and basic `*97` access readiness. Full playback IVR/navigation/save/delete, mailbox PINs, busy-to-voicemail, email, SRTP, Opus, and browser/WebRTC voicemail are future work.
