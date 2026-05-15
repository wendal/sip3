# SIP3 生产排障与发布经验（v1.2.1）

## 1. 真实根因（按出现顺序）
- `Linphone -> 网页` 方向缺少反向 WebRTC 桥接，出现“接听后秒断”。
- `sip_messages` 表在生产库缺失，导致 MESSAGE 持久化失败（1146）。
- 两个 Linphone 账号发生 NAT 端口漂移，注册表 `source_port` 与实际发消息/发呼端口不一致，造成“消息/呼叫路由到旧端口”。
- RTP relay 发包端口与 SDP 宣告端口不一致，导致接通后无音频（Linphone 会丢弃不匹配源端口媒体）。
- 两个不同公网 IP 的 Linphone 可发 MESSAGE 但收不到呼叫，根因是 REGISTER 的 Contact 带私网地址，INVITE 重拼时把私网 Contact 放进 Request-URI，部分终端会丢弃；同时 SDP 中 `public_ip` 不能使用域名。

## 2. 代码修复策略
- `proxy.rs`：补齐 SIP 主叫到 Web 被叫的反向桥接路径。
- `handler.rs` + `webrtc_gateway.rs`：按呼叫方向处理 200 INVITE SDP 与 answer 应用。
- `proxy.rs`：CANCEL 改为按已转发 INVITE 的目标 URI 与代理分支构造，避免事务不匹配。
- `proxy.rs`：对合法来源 INVITE/MESSAGE 增加注册源端口自愈（同 IP 且端口变化时刷新 `sip_registrations.source_port`）。
- `registrar.rs`：REGISTER 成功后将 Contact 改写为实际来包的公网 `source_ip:source_port`，保留 `transport=udp` 等 URI 参数。
- `proxy.rs`：非 WebSocket 被叫的 INVITE Request-URI 使用注册来源公网地址，代理新增 Via 带 `rport`；WebSocket Contact 仍保持原 URI。
- `media.rs`：RTP 转发改为“交叉 socket 发包”，保证端点收到的媒体源端口与 SDP 一致。
- `migrations/010_sip_messages.sql`：补齐 MESSAGE 存储表迁移，避免环境漏表。
- `docker-compose.yml` / `config.toml.example`：`SIP3__SERVER__PUBLIC_IP` 必须写数字公网 IPv4（当前生产为 `154.8.159.79`），不要写 `sip.air32.cn`。

## 3. 线上排障方法（高价值）
- 必须用 **Call-ID** 贯穿 SIP 日志、抓包、CDR，避免错配不同通话。
- 先证据后改动：先确认 INVITE/200/ACK/BYE/CANCEL 完整链路，再改逻辑。
- “不响铃/消息失败”先查注册路由（IP/Port/Expires）是否与实时来包一致。
- “MESSAGE 通但 INVITE 不响铃”重点查 INVITE 首行 Request-URI 是否仍指向被叫私网 Contact，以及代理 Via 是否带 `rport`。
- “已接通无声”不能只看 SIP 成功，必须抓 RTP 并验证端口方向与源端口一致性。

## 4. 发布与验证清单
- 本地：优先执行仓库根目录 `pwsh ./scripts/local-ci.ps1`（覆盖 backend `cargo fmt --check/build/test/clippy` + frontend `npm ci/build`）。
- 线上：`docker compose up -d --build backend`、`/api/health`、关键日志无异常。
- 业务验收最少覆盖：
  1. MESSAGE 双向收发与入库；
  2. 双向呼叫（1001->1003、1003->1001）；
  3. 提前挂断、拒接、正常接通后双向语音。
