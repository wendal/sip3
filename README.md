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
- ✅ SIP BYE / CANCEL / INFO bidirectional routing
- ✅ SIP REFER + NOTIFY — blind call transfer
- ✅ SIP SUBSCRIBE / NOTIFY — Presence & BLF (busy lamp field)
- ✅ SIP OPTIONS — keep-alive / capability probe (inline 200 OK)

#### Transport
- ✅ **SIP/UDP** — port 5060 (default)
- ✅ **SIP/TLS** — port 5061 (native-tls, OS cert chain)
- ✅ **SIP/WS** — port 5080 (WebSocket, for browser SIP clients)
- ✅ **SIP/WSS** — port 5443 (WebSocket Secure, reuses TLS cert)

#### Media
- ✅ **Server-side RTP relay** — NAT traversal without client STUN/TURN
- ✅ **SRTP transparent relay** — SDES/SAVP passthrough (end-to-end encryption)
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
| GET    | /api/stats                 | Dashboard statistics                  |
| GET    | /api/acl                   | List IP ACL rules                     |
| POST   | /api/acl                   | Create ACL rule                       |
| PUT    | /api/acl/:id               | Update ACL rule                       |
| DELETE | /api/acl/:id               | Delete ACL rule                       |
| GET    | /api/auth/me               | Current admin user info               |
| POST   | /api/auth/change-password  | Change admin password                 |

### Configuration

| Environment Variable              | Default        | Description                        |
|-----------------------------------|----------------|------------------------------------|
| SIP3__SERVER__SIP_HOST            | 0.0.0.0        | SIP bind address                   |
| SIP3__SERVER__SIP_PORT            | 5060           | SIP UDP port                       |
| SIP3__SERVER__SIP_DOMAIN          | sip.air32.cn   | SIP domain / registrar realm       |
| SIP3__SERVER__PUBLIC_IP           | sip.air32.cn   | Public IP written into SDP c= lines|
| SIP3__SERVER__RTP_PORT_MIN        | 10000          | RTP relay port range start         |
| SIP3__SERVER__RTP_PORT_MAX        | 10099          | RTP relay port range end           |
| SIP3__SERVER__TLS_CERT            | (empty)        | Path to TLS cert (PEM fullchain)   |
| SIP3__SERVER__TLS_KEY             | (empty)        | Path to TLS private key (PEM)      |
| SIP3__SERVER__WS_PORT             | 5080           | SIP/WS port (0 = disabled)         |
| SIP3__SERVER__WSS_PORT            | 5443           | SIP/WSS port (0 = disabled)        |
| SIP3__SERVER__WEBRTC_PORT_MIN     | 20000          | WebRTC ICE port range start        |
| SIP3__SERVER__WEBRTC_PORT_MAX     | 20099          | WebRTC ICE port range end          |
| SIP3__DATABASE__URL               | mysql://...    | MySQL connection URL                |
| SIP3__AUTH__REALM                 | sip.air32.cn   | Digest auth realm                  |
| SIP3__AUTH__REGISTRATION_EXPIRES  | 3600           | Default registration TTL (seconds) |
| SIP3__TURN__REALM                 | sip.air32.cn   | TURN realm                         |
| SIP3__TURN__SECRET                | (empty)        | TURN HMAC-SHA1 shared secret       |
| SIP3__TURN__TTL_SECONDS           | 86400          | TURN credential lifetime (seconds) |
| SIP3__TURN__SERVER                | (empty)        | TURN server URI (returned to client)|

### Development

```bash
# Backend (Rust)
cd backend
cargo build
cargo test          # 23 tests
cargo clippy -- -D warnings

# Frontend (Vue 3)
cd frontend
npm install
npm run dev         # dev server on :5173
```

See [docs/deployment.md](docs/deployment.md) for full deployment and TLS setup guide.

---

## Chinese

### 概述

SIP3 是一个功能完整的 SIP 代理/注册服务器，使用 Rust 构建后端，Vue 3 构建管理界面，支持 WebRTC 浏览器网关和 TURN 服务。

### 功能特性

#### SIP 核心
- ✅ SIP REGISTER — RFC 3261 MD5 摘要认证
- ✅ SIP INVITE 代理/B2BUA — 查找注册、重写 SDP、中继 RTP
- ✅ SIP BYE / CANCEL / INFO 双向路由
- ✅ SIP REFER + NOTIFY — 盲转呼叫
- ✅ SIP SUBSCRIBE / NOTIFY — 在线状态与 BLF（忙灯显示）
- ✅ SIP OPTIONS — 保活/能力探测

#### 传输层
- ✅ **SIP/UDP** — 5060 端口
- ✅ **SIP/TLS** — 5061 端口（系统证书链）
- ✅ **SIP/WS** — 5080 端口（WebSocket，供浏览器 SIP 客户端使用）
- ✅ **SIP/WSS** — 5443 端口（WebSocket + TLS）

#### 媒体
- ✅ **服务端 RTP 中继** — NAT 穿透，客户端无需 STUN/TURN
- ✅ **SRTP 透明中继** — SDES/SAVP 直通，端到端加密
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
| 用户名     | alice             |
| 密码       | password123       |

浏览器用户访问 `/phone` 页面，输入 SIP 账号密码即可直接通话。

### 开发说明

```bash
# 后端构建和测试
cd backend
cargo build
cargo test          # 23 个测试用例
cargo clippy -- -D warnings

# 前端开发
cd frontend
npm install
npm run dev         # 开发服务器 :5173
```

详细部署和 TLS 配置指南请参阅 [docs/deployment.md](docs/deployment.md)。
