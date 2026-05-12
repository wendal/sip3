# SIP3 — SIP Server

A production-ready SIP server with Rust backend and Vue 3 admin UI.

[English](#english) | [中文](#chinese)

---

## English

### Overview

SIP3 is a SIP proxy/registrar server built with:
- **Backend**: Rust (Tokio async, Axum REST API, SQLx + MySQL)
- **Frontend**: Vue 3 + Element Plus admin dashboard
- **Protocol**: SIP/2.0 over UDP (RFC 3261) with MD5 Digest Authentication
- **Deployment**: Docker + Docker Compose

### Features

- ✅ SIP REGISTER with RFC 3261 Digest MD5 Authentication
- ✅ SIP INVITE proxy/relay — looks up registration and forwards calls
- ✅ Multi-account support via MySQL database
- ✅ Call Detail Records (CDR) tracking
- ✅ REST API for account management
- ✅ Vue 3 admin dashboard (Dashboard, Accounts, Status pages)
- ✅ Docker + docker-compose one-command deployment
- ✅ GitHub Actions CI/CD

### Quick Start

```bash
git clone https://github.com/your-org/sip3.git
cd sip3
docker compose up -d
```

Open http://localhost to access the admin UI.
SIP server listens on UDP port 5060.

### Architecture

```
SIP Client ──UDP 5060──► SIP Server (Rust)
                               │
                         ┌─────▼─────┐
                         │ MySQL 8.0 │
                         └─────▲─────┘
                               │
Admin UI ──HTTP 80──► Nginx ──► REST API :3000
```

### API Endpoints

| Method | Path                    | Description              |
|--------|-------------------------|--------------------------|
| GET    | /api/health             | Health check             |
| GET    | /api/accounts           | List all SIP accounts    |
| POST   | /api/accounts           | Create SIP account       |
| PUT    | /api/accounts/:id       | Update SIP account       |
| DELETE | /api/accounts/:id       | Delete SIP account       |
| GET    | /api/registrations      | List active registrations|
| GET    | /api/calls              | List call records        |

### Configuration

| Variable                    | Default              | Description            |
|-----------------------------|----------------------|------------------------|
| SIP3__SERVER__SIP_HOST      | 0.0.0.0              | SIP bind address       |
| SIP3__SERVER__SIP_PORT      | 5060                 | SIP UDP port           |
| SIP3__SERVER__SIP_DOMAIN    | sip.example.com      | SIP domain             |
| SIP3__SERVER__API_PORT      | 3000                 | REST API port          |
| SIP3__DATABASE__URL         | mysql://...          | MySQL connection URL   |
| SIP3__AUTH__REALM           | sip.example.com      | Digest auth realm      |
| SIP3__AUTH__REGISTRATION_EXPIRES | 3600           | Default reg TTL (sec)  |

### Development

```bash
# Backend
cd backend
cargo build
cargo test

# Frontend
cd frontend
npm install
npm run dev
```

See [docs/deployment.md](docs/deployment.md) for full deployment guide.

---

## Chinese

### 概述

SIP3 是一个生产就绪的 SIP 服务器，使用 Rust 构建后端，Vue 3 构建管理界面。

### 功能特性

- ✅ SIP REGISTER 支持 RFC 3261 MD5 摘要认证
- ✅ SIP INVITE 代理/中继 — 查找注册信息并转发通话
- ✅ 通过 MySQL 数据库支持多账户
- ✅ 通话详细记录（CDR）跟踪
- ✅ 账户管理 REST API
- ✅ Vue 3 管理面板（仪表盘、账户、状态页面）
- ✅ Docker + docker-compose 一键部署
- ✅ GitHub Actions CI/CD

### 快速开始

```bash
git clone https://github.com/your-org/sip3.git
cd sip3
docker compose up -d
```

访问 http://localhost 打开管理界面。
SIP 服务器监听 UDP 5060 端口。

### 架构说明

```
SIP 客户端 ──UDP 5060──► SIP 服务器 (Rust)
                               │
                         ┌─────▼─────┐
                         │ MySQL 8.0 │
                         └─────▲─────┘
                               │
管理界面 ──HTTP 80──► Nginx ──► REST API :3000
```

### SIP 客户端配置

在 Linphone、Zoiper 等 SIP 客户端中配置：

| 字段     | 值                  |
|---------|---------------------|
| SIP 服务器 | 服务器 IP           |
| 端口     | 5060                |
| 协议     | UDP                 |
| 域名     | sip.example.com     |
| 用户名   | alice               |
| 密码     | password123         |

### 数据库表

- **sip_accounts**: SIP 账户（用户名、密码哈希、域名）
- **sip_registrations**: 当前活动注册（联系 URI、过期时间）
- **sip_calls**: 通话记录/CDR

### 开发说明

```bash
# 后端构建和测试
cd backend
cargo build
cargo test

# 前端开发
cd frontend
npm install
npm run dev
```

详细部署指南请参阅 [docs/deployment.md](docs/deployment.md)。
