# SIP3 Deployment Guide

## Prerequisites

- Docker Engine 24+ and Docker Compose v2
- MySQL 8.0 (if deploying without Docker)
- Rust 1.82+ (for building from source)
- Node.js 20+ (for frontend from source)

## Quick Start with Docker Compose

```bash
# Clone the repository
git clone https://github.com/your-org/sip3.git
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
- REST API on port 3000
- Admin UI on port 80

## Manual Deployment

### 1. Database Setup

```sql
CREATE DATABASE sip3;
CREATE USER 'sip3'@'%' IDENTIFIED BY 'sip3pass';
GRANT ALL PRIVILEGES ON sip3.* TO 'sip3'@'%';
FLUSH PRIVILEGES;
```

Then run migrations:
```bash
mysql -u sip3 -p sip3 < migrations/001_initial.sql
mysql -u sip3 -p sip3 < migrations/002_seed.sql
```

### 2. Backend Configuration

Create `backend/config.toml`:
```toml
[server]
sip_host = "0.0.0.0"
sip_port = 5060
sip_domain = "your-domain.com"
api_host = "0.0.0.0"
api_port = 3000

[database]
url = "mysql://sip3:sip3pass@localhost:3306/sip3"
max_connections = 10

[auth]
realm = "your-domain.com"
registration_expires = 3600
```

Or set environment variables (prefix `SIP3__`):
```bash
export SIP3__SERVER__SIP_DOMAIN=your-domain.com
export SIP3__DATABASE__URL=mysql://sip3:sip3pass@localhost:3306/sip3
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

## SIP Client Configuration

Configure your SIP client (Linphone, Zoiper, etc.):

| Field       | Value                    |
|-------------|--------------------------|
| SIP Server  | your-server-ip           |
| Port        | 5060                     |
| Protocol    | UDP                      |
| Domain      | your-domain.com          |
| Username    | alice (or other user)    |
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
  -d '{"username":"dave","password":"secret","display_name":"Dave","domain":"your-domain.com"}'
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
UDP 5060   - SIP signaling
TCP 3000   - REST API (internal only in production)
TCP 80/443 - Admin UI
TCP 3306   - MySQL (internal only)
```

## Production Hardening

1. **TLS/SRTP**: Use a SIP proxy (Kamailio/OpenSIPS) with TLS in front for encrypted signaling
2. **Firewall**: Restrict SIP port to trusted IP ranges
3. **Rate limiting**: Add nginx rate limiting for the API
4. **Database**: Use strong passwords and bind MySQL to 127.0.0.1
5. **Secrets**: Use Docker secrets or a vault for passwords
6. **Logging**: Configure log aggregation (ELK, Loki, etc.)

## Troubleshooting

### SIP registration fails
- Verify the SIP domain matches `auth.realm` in config
- Check account exists: `curl http://localhost:3000/api/accounts`
- Ensure UDP 5060 is reachable from the client

### Database connection fails
- Check `DATABASE_URL` is correct
- Verify MySQL is running: `docker compose ps mysql`
- Check credentials: `mysql -u sip3 -psip3pass -h localhost sip3`

### Build fails
- Rust: ensure stable toolchain `rustup update stable`
- Frontend: clear cache `rm -rf frontend/node_modules && npm ci`
