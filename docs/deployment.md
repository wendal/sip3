# SIP3 Deployment Guide

## Prerequisites

- Docker Engine 24+ and Docker Compose v2
- MySQL 8.0 (if deploying without Docker)
- Rust 1.95+ (for building from source; required for Rust 2024 edition)
- Node.js 20+ (for frontend from source)

## Quick Start with Docker Compose

```bash
# Clone the repository
git clone https://github.com/wendal/sip3.git
cd sip3

# Start all services
docker compose up -d

# View logs
docker compose logs -f

# Check health
curl http://localhost:3000/api/health
```

Services started:
- MySQL on port 3306
- SIP server on UDP port 5060
- RTP media relay on UDP ports 10000–10099
- REST API on port 3000
- Admin UI on port 8030

## Manual Deployment

### 1. Database Setup

```sql
CREATE DATABASE sip3;
-- Using root or create a dedicated user:
-- CREATE USER 'sip3'@'%' IDENTIFIED BY 'sip3pass';
-- GRANT ALL PRIVILEGES ON sip3.* TO 'sip3'@'%';
-- FLUSH PRIVILEGES;
```

Then run migrations:
```bash
mysql -u root -proot sip3 < migrations/001_initial.sql
mysql -u root -proot sip3 < migrations/002_seed.sql
mysql -u root -proot sip3 < migrations/003_media_sessions.sql
```

### 2. Backend Configuration

Create `backend/config.toml`:
```toml
[server]
sip_host = "0.0.0.0"
sip_port = 5060
sip_domain = "sip.air32.cn"
api_host = "0.0.0.0"
api_port = 3000
# Public IPv4 of this server written into rewritten SDP c= lines
public_ip = "154.8.159.79"
# UDP port range for RTP media relay
rtp_port_min = 10000
rtp_port_max = 20000

[database]
url = "mysql://root:root@localhost:3306/sip3"
max_connections = 10

[auth]
realm = "sip.air32.cn"
registration_expires = 3600
```

Or set environment variables (prefix `SIP3__`):
```bash
export SIP3__SERVER__SIP_DOMAIN=sip.air32.cn
export SIP3__SERVER__PUBLIC_IP=154.8.159.79
export SIP3__DATABASE__URL=mysql://root:root@localhost:3306/sip3
```

### 3. Build and Run Backend

```bash
cd backend
cargo build --release
./target/release/sip3-backend
```

### 4. Build and Serve Frontend

```bash
cd frontend
npm ci
npm run build
# Serve dist/ with any static file server, e.g.:
npx serve dist -p 80
```

## RTP Media Relay

SIP3 includes a built-in server-side RTP relay so clients **do not need STUN or TURN**.
When an INVITE is proxied, the server:

1. Allocates a pair of UDP ports from the configured `rtp_port_min`–`rtp_port_max` range.
2. Rewrites the SDP `c=` and `m=audio` fields in the INVITE to point to the server.
3. When the 200 OK arrives from the callee, rewrites its SDP the same way.
4. Learns each peer's real public RTP address from the first packet received
   (symmetric RTP – no SDP address trust required).
5. Bidirectionally forwards RTP between the two peers.

This means clients behind NAT with no fixed public IP can call each other normally.

## SIP Client Configuration

Configure your SIP client (Linphone, Zoiper, etc.):

| Field       | Value                    |
|-------------|--------------------------|
| SIP Server  | sip.air32.cn             |
| Port        | 5060                     |
| Protocol    | UDP                      |
| Domain      | sip.air32.cn             |
| Username    | 1001 (or other extension) |
| Password    | password123 (from seed)  |
| Auth Method | Digest MD5               |

## Managing Accounts via API

### List accounts
```bash
curl http://localhost:3000/api/accounts
```

### Create account
```bash
curl -X POST http://localhost:3000/api/accounts \
  -H 'Content-Type: application/json' \
  -d '{"username":"dave","password":"secret","display_name":"Dave","domain":"sip.air32.cn"}'
```

### Update account
```bash
curl -X PUT http://localhost:3000/api/accounts/1 \
  -H 'Content-Type: application/json' \
  -d '{"enabled":0}'
```

### Delete account
```bash
curl -X DELETE http://localhost:3000/api/accounts/1
```

### List active registrations
```bash
curl http://localhost:3000/api/registrations
```

### List call records
```bash
curl http://localhost:3000/api/calls
```

## Firewall Rules

Open these ports:
```
UDP 5060         - SIP signaling
UDP 10000-10099  - RTP media relay
TCP 3000         - REST API (internal only in production)
TCP 8030/443     - Admin UI
TCP 3306         - MySQL (internal only)
```

## Production Hardening

1. **TLS/SRTP**: Use a SIP proxy (Kamailio/OpenSIPS) with TLS in front for encrypted signaling
2. **Firewall**: Restrict SIP port to trusted IP ranges
3. **Rate limiting**: Add nginx rate limiting for the API
4. **Database**: Use strong passwords and bind MySQL to 127.0.0.1
5. **Secrets**: Use Docker secrets or a vault for passwords
6. **Logging**: Configure log aggregation (ELK, Loki, etc.)
7. **public_ip**: Set `server.public_ip` to the server's actual public IPv4 address
8. **Built-in bruteforce protection**: tune `SIP3__SECURITY__*` thresholds and keep `/api/security/*` endpoints behind JWT/API-key auth

## Troubleshooting

### SIP registration fails
- Verify the SIP domain matches `auth.realm` in config (`sip.air32.cn`)
- Check account exists: `curl http://localhost:3000/api/accounts`
- Ensure UDP 5060 is reachable from the client

### Repeated scan / bruteforce attempts
- Check current auto-bans: `curl -H "Authorization: Bearer <JWT>" http://localhost:3000/api/security/blocks`
- Check recent security events: `curl -H "Authorization: Bearer <JWT>" "http://localhost:3000/api/security/events?limit=100"`
- Check runtime troubleshooting snapshot: `curl -H "Authorization: Bearer <JWT>" http://localhost:3000/api/security/runtime`
- Manually unblock one CIDR if needed:
  `curl -X POST -H "Authorization: Bearer <JWT>" -H "Content-Type: application/json" -d '{"cidr":"203.0.113.10/32"}' http://localhost:3000/api/security/blocks/unblock`

### No audio after call connects
- Verify UDP ports 10000–10099 are open and forwarded to the server
- Verify `server.public_ip` is the actual public IP address visible to clients
- Check server logs for "Allocated media relay" and "RTP relay" messages

### Database connection fails
- Check `DATABASE_URL` is correct
- Verify MySQL is running: `systemctl status mysql`
- Check credentials: `mysql -u root -proot sip3`

### Build fails
- Rust: ensure stable toolchain is at least 1.95 (`rustup update stable`)
- Frontend: clear cache `rm -rf frontend/node_modules && npm ci`
