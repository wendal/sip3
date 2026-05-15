# SIP3 — SIP Server

A production-ready SIP proxy/registrar with Rust backend, Vue 3 admin UI, and WebRTC browser gateway.

[English](#english) | [中文](#chinese)

---

## English

### Overview

SIP3 is a full-featured SIP proxy/registrar server built with:
- **Backend**: Rust (Tokio async, Axum REST API, SQLx + MySQL)
- **Frontend**: Vue 3 + Element Plus admin dashboard (Chinese/English)
- **Protocols**: SIP/2.0 over UDP, TCP/TLS, WebSocket, WebSocket-Secure
- **WebRTC**: Built-in media gateway + coturn TURN server for browser clients
- **Deployment**: Docker + Docker Compose

### Features

#### SIP Core
- ✅ SIP REGISTER — RFC 3261 MD5 Digest Authentication
- ✅ SIP INVITE proxy/B2BUA — looks up registration, rewrites SDP, relays RTP
- ✅ **Audio conference rooms** — local B2BUA endpoint with server-side G.711 (PCMU/PCMA) mixer; dial e.g. `sip:900000000@<domain>`; `*6` DTMF (RFC 2833 or SIP INFO) toggles mute. MVP is SIP UDP/TLS only — no SRTP/video/WebRTC, no PIN.
- ✅ **Voicemail** — mailbox recording for offline users and unanswered calls (default 25 s), `*97` mailbox access, and MWI via `SUBSCRIBE Event: message-summary`. MVP supports G.711 PCMU/PCMA RTP/AVP only; no PIN, busy-to-voicemail, email notifications, SRTP, Opus, or browser/WebRTC voicemail.
- ✅ SIP BYE / CANCEL / INFO bidirectional routing
- ✅ SIP REFER + NOTIFY — blind call transfer
- ✅ SIP SUBSCRIBE / NOTIFY — Presence & BLF (busy lamp field)
- ✅ SIP MESSAGE — instant messaging relay + persistence
- ✅ SIP OPTIONS — keep-alive / capability probe (inline 200 OK)

#### Transport
- ✅ **SIP/UDP** — port 5060 (default)
- ✅ **SIP/TLS** — port 5061 (native-tls, OS cert chain)
- ✅ **SIP/WS** — port 5080 (WebSocket, for browser SIP clients)
- ✅ **SIP/WSS** — port 5443 (WebSocket Secure, reuses TLS cert)

#### Media
- ✅ **Server-side RTP relay** — NAT traversal for SIP phone audio and video without client STUN/TURN
- ✅ **SRTP transparent relay** — SDES/SAVP audio/video passthrough (end-to-end encryption)
- ✅ **WebRTC media gateway** — bridges browser ICE/DTLS-SRTP ↔ plain SIP phone RTP
- ✅ **TURN credentials API** — coturn HMAC-SHA1 time-limited credentials

#### Admin & Security
- ✅ IP ACL — CIDR allow/deny rules with priority, hot-reloaded every 60 s
- ✅ JWT admin authentication (login, change-password, admin user management)
- ✅ Call Detail Records (CDR) — start/end time, duration, status
- ✅ Dashboard statistics — active registrations, ongoing calls, CDR totals
- ✅ REST API — full CRUD for accounts, registrations, ACL rules, call records

#### Frontend
- ✅ Vue 3 + Element Plus admin dashboard (全中文界面)
- ✅ Browser softphone at `/phone` — SIP.js + WebRTC, TURN auto-configured
- ✅ Search, pagination, de-register, call statistics

### What's new in v1.2.0

- ✅ Added reverse WebRTC bridge for **SIP caller -> browser callee** flows.
- ✅ Fixed CANCEL forwarding to preserve transaction consistency with forwarded INVITE.
- ✅ Added sender-source registration port refresh for authenticated INVITE/MESSAGE to survive NAT port rebinding.
- ✅ Fixed RTP relay source-port behavior so media packets are sent from SDP-signaled relay ports.
- ✅ Added `migrations/010_sip_messages.sql` for MESSAGE persistence schema completeness.

### Quick Start

```bash
git clone https://github.com/wendal/sip3.git
cd sip3
docker compose up -d
```

Open **http://localhost:8030** for the admin UI (default: `admin` / `admin123`).

| Service        | Port(s)              |
|----------------|----------------------|
| Admin UI       | TCP 8030             |
| REST API       | TCP 3000             |
| SIP/UDP        | UDP 5060             |
| SIP/TLS        | TCP 5061             |
| SIP/WS         | TCP 5080             |
| SIP/WSS        | TCP 5443             |
| RTP relay      | UDP 10000–10099      |
| Conference RTP | UDP 10100–10199      |
| Voicemail RTP  | UDP 10200–10299      |
| TURN/UDP       | UDP 3478             |
| TURN/TLS       | TCP 5349             |

### Architecture

```
Browser (SIP.js)──WSS 5443──┐
SIP Phone ────────UDP 5060──┼──► SIP Handler (Rust)
SIP Phone (TLS) ──TLS 5061──┘         │
                                  ┌────▼────┐   ┌─────────────────┐
                                  │ Registrar│   │ WebRTC Gateway  │
                                  │ Proxy   │   │ (webrtc-rs B2BUA│
                                  │ Presence│   │  ICE+DTLS-SRTP) │
                                  └────┬────┘   └─────────────────┘
                                       │
                             ┌─────────▼─────────┐
                             │   MySQL 8.0        │
                             │ sip_accounts       │
                             │ sip_registrations  │
                             │ sip_calls          │
                             │ sip_acl            │
                             └─────────┬──────────┘
                                       │
Admin UI ──HTTP 8030──► Nginx ─────────► REST API :3000
```

### API Endpoints

**Public (no auth required)**

| Method | Path                       | Description                           |
|--------|----------------------------|---------------------------------------|
| GET    | /api/health                | Health check                          |
| POST   | /api/auth/login            | Admin login → JWT                     |
| POST   | /api/turn/credentials      | TURN creds (SIP HA1 auth)             |
| POST   | /api/messages/history      | Phone message history (SIP credential auth) |

**Protected (JWT required)**

| Method | Path                       | Description                           |
|--------|----------------------------|---------------------------------------|
| GET    | /api/accounts              | List SIP accounts                     |
| POST   | /api/accounts              | Create SIP account                    |
| PUT    | /api/accounts/:id          | Update SIP account                    |
| DELETE | /api/accounts/:id          | Delete SIP account                    |
| GET    | /api/registrations         | List active registrations             |
| DELETE | /api/registrations/:id     | Force de-register                     |
| GET    | /api/calls                 | List call records (CDR)               |
| POST   | /api/calls/cleanup         | Close stale active calls (`?older_than_hours=N`, default 4; pass 0 for all). Backend also runs this automatically at startup and every 5 min. |
| GET    | /api/messages              | List persisted SIP MESSAGE records    |
| GET/POST | /api/voicemail/boxes      | List/create voicemail mailboxes       |
| PUT    | /api/voicemail/boxes/:id   | Update voicemail mailbox settings     |
| GET    | /api/voicemail/messages    | List voicemail messages               |
| PUT/DELETE | /api/voicemail/messages/:id | Update or soft-delete a message    |
| GET    | /api/voicemail/messages/:id/download | Download message WAV audio |
| GET    | /api/stats                 | Dashboard statistics                  |
| GET    | /api/security/summary      | Security summary (24h failures/blocks) |
| GET    | /api/security/events       | Security event timeline               |
| GET    | /api/security/blocks       | Active auto-ban ACL entries           |
| POST   | /api/security/blocks/unblock | Disable one auto-ban entry          |
| GET    | /api/security/runtime      | Runtime troubleshooting snapshot      |
| GET    | /api/acl                   | List IP ACL rules                     |
| POST   | /api/acl                   | Create ACL rule                       |
| PUT    | /api/acl/:id               | Update ACL rule                       |
| DELETE | /api/acl/:id               | Delete ACL rule                       |
| GET    | /api/auth/me               | Current admin user info               |
| POST   | /api/auth/change-password  | Change admin password                 |


### Voicemail

- **Delivery**: if an enabled mailbox's owner is offline, SIP3 answers immediately and records a message. If the owner is registered but does not answer, SIP3 sends the call to voicemail after `SIP3__SERVER__VOICEMAIL_NO_ANSWER_SECS` (default 25 seconds).
- **Mailbox access**: users dial `*97` from their own SIP account to play mailbox prompts and messages.
- **Codecs**: voicemail accepts G.711 PCMU/PCMA on RTP/AVP only. SRTP/SAVP, Opus, video, and browser/WebRTC voicemail are not supported in the MVP.
- **MWI**: SIP phones can subscribe with `SUBSCRIBE Event: message-summary`; SIP3 sends `NOTIFY` updates with new/saved message counts.
- **Storage**: recordings are WAV files under `SIP3__SERVER__VOICEMAIL_STORAGE_DIR` (Docker default `/app/voicemail`, host mount `./voicemail`). Prompt WAV files are read from `SIP3__SERVER__VOICEMAIL_PROMPT_DIR` (default `voicemail/prompts`).
- **RTP ports**: voicemail media uses UDP `10200-10299` by default (`SIP3__SERVER__VOICEMAIL_RTP_PORT_MIN/MAX`). Open and Docker-map this range separately from call relay and conference RTP.
- **DTMF menu**: `1` replays the current message; `2` or `#` saves/skips to the next message; `7` deletes; `9` saves; `*` exits or returns to the previous menu.
- **MVP exclusions**: mailbox PINs, busy-to-voicemail routing, email notifications, SRTP, Opus, and browser/WebRTC voicemail.

### Configuration

| Environment Variable              | Default        | Description                        |
|-----------------------------------|----------------|------------------------------------|
| SIP3__SERVER__SIP_HOST            | 0.0.0.0        | SIP bind address                   |
| SIP3__SERVER__SIP_PORT            | 5060           | SIP UDP port                       |
| SIP3__SERVER__SIP_DOMAIN          | sip.air32.cn   | SIP domain / registrar realm       |
| SIP3__SERVER__PUBLIC_IP           | 154.8.159.79   | Public IPv4 written into SDP c= lines|
| SIP3__SERVER__RTP_PORT_MIN        | 10000          | RTP relay port range start         |
| SIP3__SERVER__RTP_PORT_MAX        | 10099          | RTP relay port range end           |
| SIP3__SERVER__CONFERENCE_RTP_PORT_MIN | 10100      | Conference RTP port range start    |
| SIP3__SERVER__CONFERENCE_RTP_PORT_MAX | 10199      | Conference RTP port range end      |
| SIP3__SERVER__VOICEMAIL_ACCESS_EXTENSION | *97       | Voicemail mailbox access extension |
| SIP3__SERVER__VOICEMAIL_NO_ANSWER_SECS | 25         | Seconds before no-answer voicemail |
| SIP3__SERVER__VOICEMAIL_MAX_MESSAGE_SECS | 120       | Maximum voicemail recording length |
| SIP3__SERVER__VOICEMAIL_IDLE_TIMEOUT_SECS | 10       | Stop recording after RTP silence   |
| SIP3__SERVER__VOICEMAIL_STORAGE_DIR | voicemail      | Directory for voicemail WAV files  |
| SIP3__SERVER__VOICEMAIL_PROMPT_DIR | voicemail/prompts | Directory for voicemail prompt WAVs |
| SIP3__SERVER__VOICEMAIL_RTP_PORT_MIN | 10200       | Voicemail RTP port range start     |
| SIP3__SERVER__VOICEMAIL_RTP_PORT_MAX | 10299       | Voicemail RTP port range end       |
| SIP3__SERVER__TLS_CERT            | (empty)        | Path to TLS cert (PEM fullchain)   |
| SIP3__SERVER__TLS_KEY             | (empty)        | Path to TLS private key (PEM)      |
| SIP3__SERVER__WS_PORT             | 5080           | SIP/WS port (0 = disabled)         |
| SIP3__SERVER__WSS_PORT            | 5443           | SIP/WSS port (0 = disabled)        |
| SIP3__SERVER__WEBRTC_PORT_MIN     | 20000          | WebRTC ICE port range start        |
| SIP3__SERVER__WEBRTC_PORT_MAX     | 20099          | WebRTC ICE port range end          |
| SIP3__DATABASE__URL               | mysql://...    | MySQL connection URL                |
| SIP3__AUTH__REALM                 | sip.air32.cn   | Digest auth realm                  |
| SIP3__AUTH__REGISTRATION_EXPIRES  | 3600           | Default registration TTL (seconds) |
| SIP3__SECURITY__WINDOW_SECS       | 300            | Sliding window for auth-fail counters |
| SIP3__SECURITY__SIP_IP_FAIL_THRESHOLD | 20        | REGISTER failures/IP before block      |
| SIP3__SECURITY__SIP_USER_IP_FAIL_THRESHOLD | 8    | REGISTER failures/IP+user before block |
| SIP3__SECURITY__API_IP_FAIL_THRESHOLD | 20        | Admin login failures/IP before block    |
| SIP3__SECURITY__API_USER_IP_FAIL_THRESHOLD | 8    | Admin login failures/IP+user before block |
| SIP3__SECURITY__BLOCK_SECS        | 900            | Auto-ban duration (seconds)            |
| SIP3__SECURITY__PERSIST_ACL_BANS  | true           | Persist auto-ban into `sip_acl`        |
| SIP3__SECURITY__ACL_BAN_PRIORITY  | 5              | Priority for auto-ban ACL rules        |
| SIP3__TURN__REALM                 | sip.air32.cn   | TURN realm                         |
| SIP3__TURN__SECRET                | (empty)        | TURN HMAC-SHA1 shared secret       |
| SIP3__TURN__TTL_SECONDS           | 86400          | TURN credential lifetime (seconds) |
| SIP3__TURN__SERVER                | (empty)        | TURN server URI (returned to client)|

> **Important**: set `SIP3__SERVER__PUBLIC_IP` to a **numeric public IPv4** (for example `154.8.159.79`) to avoid SIP endpoint compatibility issues caused by domain names inside SDP `c=IN IP4`.

### Development

```bash
# Local CI parity check (recommended before push)
pwsh ./scripts/local-ci.ps1

# Backend (Rust)
cd backend
cargo fmt --check
cargo build
cargo test
cargo clippy -- -D warnings

# Frontend (Vue 3)
cd frontend
npm install
npm run dev         # dev server on :5173
```

### Troubleshooting

- **No ringing / MESSAGE not delivered**: check `sip_registrations.source_ip/source_port` against the sender's real source socket.
- **Call connected but no audio**: verify UDP RTP range (`10000-10099`) is open and relay ports in SDP match packet source ports.
- **Linphone video does not appear**: verify both offer and answer SDP rewrite `m=video` to the SIP3 public IP and a relay port. A SIP audio+video call consumes four RTP relay ports, so the default range supports fewer concurrent video calls than audio-only calls. Browser WebRTC video is not covered by the legacy SIP RTP relay path.
- **MESSAGE persistence errors (1146)**: ensure migration `010_sip_messages.sql` has been applied.

See [docs/deployment.md](docs/deployment.md) for full deployment and TLS setup guide.

---

## Chinese

### 概述

SIP3 是一个功能完整的 SIP 代理/注册服务器，使用 Rust 构建后端，Vue 3 构建管理界面，支持 WebRTC 浏览器网关和 TURN 服务。

### 功能特性

#### SIP 核心
- ✅ SIP REGISTER — RFC 3261 MD5 摘要认证
- ✅ SIP INVITE 代理/B2BUA — 查找注册、重写 SDP、中继 RTP
- ✅ **音频会议室** — 本地 B2BUA 端点，服务端 G.711 (PCMU/PCMA) 混音；拨打如 `sip:900000000@<域>` 入会；`*6` DTMF（RFC 2833 或 SIP INFO）切换静音。MVP 仅支持 SIP UDP/TLS，不支持 SRTP/视频/WebRTC，无 PIN。
- ✅ **语音信箱** — 离线用户和无人接听（默认 25 秒）转入本地录音；用户拨打 `*97` 进入信箱；支持 `SUBSCRIBE Event: message-summary` 消息等待指示（MWI）。MVP 仅支持 G.711 PCMU/PCMA RTP/AVP，不支持 PIN、忙线转信箱、邮件通知、SRTP、Opus 或浏览器/WebRTC 语音信箱。
- ✅ SIP BYE / CANCEL / INFO 双向路由
- ✅ SIP REFER + NOTIFY — 盲转呼叫
- ✅ SIP SUBSCRIBE / NOTIFY — 在线状态与 BLF（忙灯显示）
- ✅ SIP MESSAGE — 即时消息转发与持久化
- ✅ SIP OPTIONS — 保活/能力探测

#### 传输层
- ✅ **SIP/UDP** — 5060 端口
- ✅ **SIP/TLS** — 5061 端口（系统证书链）
- ✅ **SIP/WS** — 5080 端口（WebSocket，供浏览器 SIP 客户端使用）
- ✅ **SIP/WSS** — 5443 端口（WebSocket + TLS）

#### 媒体
- ✅ **服务端 RTP 中继** — SIP 电话音频和视频 NAT 穿透，客户端无需 STUN/TURN
- ✅ **SRTP 透明中继** — SDES/SAVP 音视频直通，端到端加密
- ✅ **WebRTC 媒体网关** — 浏览器 ICE/DTLS-SRTP ↔ 传统 SIP 电话 RTP 互通
- ✅ **TURN 凭证 API** — coturn HMAC-SHA1 时效凭证，浏览器端自动获取

#### 管理与安全
- ✅ IP ACL — CIDR 允许/拒绝规则，优先级，每 60 秒热重载
- ✅ JWT 管理员认证（登录、改密、管理员用户管理）
- ✅ 通话详细记录（CDR）— 开始/结束时间、时长、状态
- ✅ 仪表盘统计 — 活跃注册数、进行中通话数、CDR 汇总
- ✅ REST API — 账户、注册、ACL、通话记录完整 CRUD

#### 前端
- ✅ Vue 3 + Element Plus 全中文管理界面
- ✅ `/phone` 浏览器软电话 — SIP.js + WebRTC，自动获取 TURN 凭证
- ✅ 搜索、分页、强制注销、通话统计

### v1.2.0 更新亮点

- ✅ 新增 **SIP 主叫 -> 浏览器被叫** 的反向 WebRTC 桥接。
- ✅ 修复 CANCEL 转发事务一致性（与已转发 INVITE 保持一致）。
- ✅ 新增已认证 INVITE/MESSAGE 的注册源端口自愈，缓解 NAT 端口漂移问题。
- ✅ 修复 RTP relay 源端口行为，确保媒体源端口与 SDP 宣告端口一致。
- ✅ 新增 `migrations/010_sip_messages.sql`，补齐 MESSAGE 存储表迁移。

### 快速开始

```bash
git clone https://github.com/wendal/sip3.git
cd sip3
docker compose up -d
```

访问 **http://localhost:8030** 打开管理界面（默认账户：`admin` / `admin123`）。

| 服务        | 端口              |
|------------|-------------------|
| 管理界面    | TCP 8030          |
| REST API   | TCP 3000          |
| SIP/UDP    | UDP 5060          |
| SIP/TLS    | TCP 5061          |
| SIP/WS     | TCP 5080          |
| SIP/WSS    | TCP 5443          |
| RTP 中继   | UDP 10000–10099   |
| 会议 RTP   | UDP 10100–10199   |
| 语音信箱 RTP | UDP 10200–10299 |
| TURN/UDP   | UDP 3478          |
| TURN/TLS   | TCP 5349          |

### 架构说明

```
浏览器(SIP.js)──WSS 5443──┐
SIP 电话 ─────────UDP 5060──┼──► SIP 处理器 (Rust)
SIP 电话(TLS) ──TLS 5061──┘         │
                                ┌────▼────┐   ┌──────────────────┐
                                │ 注册器  │   │ WebRTC 媒体网关  │
                                │ 代理    │   │ (webrtc-rs B2BUA │
                                │ 在线状态│   │  ICE+DTLS-SRTP)  │
                                └────┬────┘   └──────────────────┘
                                     │
                           ┌─────────▼──────────┐
                           │   MySQL 8.0        │
                           └─────────┬──────────┘
                                     │
管理界面 ──HTTP 8030──► Nginx ────────► REST API :3000
```

### SIP 客户端配置

在 Linphone、Zoiper 等 SIP 客户端中配置：

| 字段       | 值                |
|-----------|-------------------|
| SIP 服务器 | sip.air32.cn      |
| 端口       | 5060 (UDP) / 5061 (TLS) |
| 协议       | UDP / TLS / WebSocket |
| 域名       | sip.air32.cn      |
| 用户名     | 1001              |
| 密码       | password123       |

浏览器用户访问 `/phone` 页面，输入 SIP 账号密码即可直接通话。


### 语音信箱

- **投递规则**：启用信箱的用户离线时立即接入语音信箱；在线但无人接听时，默认 25 秒后接入（`SIP3__SERVER__VOICEMAIL_NO_ANSWER_SECS`）。
- **信箱访问**：用户从自己的 SIP 账号拨打 `*97` 收听提示音和留言。
- **编解码**：仅支持 G.711 PCMU/PCMA RTP/AVP；MVP 不支持 SRTP/SAVP、Opus、视频或浏览器/WebRTC 语音信箱。
- **MWI**：终端可发送 `SUBSCRIBE Event: message-summary` 订阅消息等待指示，SIP3 通过 `NOTIFY` 推送新/已保存留言数量。
- **存储目录**：留言以 WAV 文件保存在 `SIP3__SERVER__VOICEMAIL_STORAGE_DIR`（Docker 默认 `/app/voicemail`，宿主机挂载 `./voicemail`）；提示音从 `SIP3__SERVER__VOICEMAIL_PROMPT_DIR` 读取（默认 `voicemail/prompts`）。
- **RTP 端口**：语音信箱媒体默认使用 UDP `10200-10299`（`SIP3__SERVER__VOICEMAIL_RTP_PORT_MIN/MAX`），需单独放行并映射。
- **DTMF 菜单**：`1` 重播当前留言；`2` 或 `#` 保存/跳到下一条；`7` 删除；`9` 保存；`*` 退出或返回上级菜单。
- **MVP 不包含**：信箱 PIN、忙线转信箱、邮件通知、SRTP、Opus、浏览器/WebRTC 语音信箱。

### 开发说明

```bash
# 本地 CI 对齐检查（推荐在 push 前执行）
pwsh ./scripts/local-ci.ps1

# 后端构建和测试
cd backend
cargo fmt --check
cargo build
cargo test
cargo clippy -- -D warnings

# 前端开发
cd frontend
npm install
npm run dev         # 开发服务器 :5173
```

### 常见问题排查

- **不响铃 / MESSAGE 收不到**：优先核对 `sip_registrations` 中 `source_ip/source_port` 是否与实时来包一致。
- **接通无声音**：确认 UDP `10000-10099` 已放行，并核对 SDP 中继端口与实际 RTP 源端口一致。
- **Linphone 视频无画面**：确认 INVITE 和 200 OK 的 SDP 都把 `m=video` 改写到 SIP3 公网 IP 和中继端口。SIP 音视频通话每路会占用 4 个 RTP 中继端口，因此默认端口范围支持的并发视频通话少于纯音频通话。浏览器 WebRTC 视频不属于传统 SIP RTP 中继路径。
- **MESSAGE 入库报 1146**：确认已执行迁移 `010_sip_messages.sql`。

> **重要**：`SIP3__SERVER__PUBLIC_IP` 建议配置为公网 **数字 IPv4**（例如 `154.8.159.79`），避免终端因 SDP 中使用域名导致兼容问题。

详细部署和 TLS 配置指南请参阅 [docs/deployment.md](docs/deployment.md)。
