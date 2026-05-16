# SIP3 生产排障与发布经验（v1.3.0）

## 1. 真实根因（按出现顺序）
- `Linphone -> 网页` 方向缺少反向 WebRTC 桥接，出现“接听后秒断”。
- `sip_messages` 表在生产库缺失，导致 MESSAGE 持久化失败（1146）。
- 两个 Linphone 账号发生 NAT 端口漂移，注册表 `source_port` 与实际发消息/发呼端口不一致，造成“消息/呼叫路由到旧端口”。
- RTP relay 发包端口与 SDP 宣告端口不一致，导致接通后无音频（Linphone 会丢弃不匹配源端口媒体）。
- 两个不同公网 IP 的 Linphone 可发 MESSAGE 但收不到呼叫，根因是 REGISTER 的 Contact 带私网地址，INVITE 重拼时把私网 Contact 放进 Request-URI，部分终端会丢弃；同时 SDP 中 `public_ip` 不能使用域名。
- Linphone 音频正常但视频无画面，根因是 RTP relay 只为首个 `m=audio` 分配端口并重写 SDP，`m=video` 仍指向终端原始/私网地址且没有服务端视频 relay socket。
- 会议和语音信箱不能复用通话 RTP relay：会议需要解码/混音，语音信箱需要录音/播放提示音，必须作为本地 SIP 端点独立分配 RTP 端口。
- 语音信箱的 `*97` 和 MWI 订阅如果只信任 From 头会被伪造；当前实现要求请求来源 IP/端口匹配有效注册源。

## 2. 代码修复策略
- `proxy.rs`：补齐 SIP 主叫到 Web 被叫的反向桥接路径。
- `handler.rs` + `webrtc_gateway.rs`：按呼叫方向处理 200 INVITE SDP 与 answer 应用。
- `proxy.rs`：CANCEL 改为按已转发 INVITE 的目标 URI 与代理分支构造，避免事务不匹配。
- `proxy.rs`：对合法来源 INVITE/MESSAGE 增加注册源端口自愈（同 IP 且端口变化时刷新 `sip_registrations.source_port`）。
- `registrar.rs`：REGISTER 成功后将 Contact 改写为实际来包的公网 `source_ip:source_port`，保留 `transport=udp` 等 URI 参数。
- `proxy.rs`：非 WebSocket 被叫的 INVITE Request-URI 使用注册来源公网地址，代理新增 Via 带 `rport`；WebSocket Contact 仍保持原 URI。
- `media.rs`：RTP 转发改为“交叉 socket 发包”，保证端点收到的媒体源端口与 SDP 一致。
- `media.rs` + `proxy.rs` + `handler.rs`：传统 SIP 电话通话按 SDP 中 active `m=audio`/`m=video` section 分配独立双向 RTP relay，INVITE 使用 callee-facing 端口，200 OK 使用 caller-facing 端口；保留 `a=rtpmap`/`a=fmtp`/`a=rtcp-fb`/`a=crypto`，`m=video 0` 不分配端口。
- `migrations/010_sip_messages.sql`：补齐 MESSAGE 存储表迁移，避免环境漏表。
- `docker-compose.yml` / `backend/config.toml`：`SIP3__SERVER__PUBLIC_IP` 必须写数字公网 IPv4（当前生产为 `154.8.159.79`），不要写 `sip.air32.cn`。
- `conference.rs` / `conference_media.rs` / `conference_sdp.rs`：会议室作为 9 位数字本地端点，协商 RTP/AVP PCMU/PCMA，按参与者分配 `10100-10199` RTP 端口并做服务端混音。
- `voicemail.rs` / `voicemail_media.rs` / `voicemail_mwi.rs`：语音信箱作为本地端点处理离线/无人接听录音、`*97` 基础访问、MWI 订阅/通知，使用 `10200-10299` RTP 端口。
- `storage/voicemail.rs`：留言文件使用 UUID 后缀和 create-new 语义，避免重复 Call-ID 覆盖已有 WAV。
- `api/voicemail.rs`：管理端改留言状态/删除后触发 MWI 刷新。

## 3. 线上排障方法（高价值）
- 必须用 **Call-ID** 贯穿 SIP 日志、抓包、CDR，避免错配不同通话。
- 先证据后改动：先确认 INVITE/200/ACK/BYE/CANCEL 完整链路，再改逻辑。
- “不响铃/消息失败”先查注册路由（IP/Port/Expires）是否与实时来包一致。
- “MESSAGE 通但 INVITE 不响铃”重点查 INVITE 首行 Request-URI 是否仍指向被叫私网 Contact，以及代理 Via 是否带 `rport`。
- “已接通无声”不能只看 SIP 成功，必须抓 RTP 并验证端口方向与源端口一致性。
- “音频正常但视频无画面”重点查 INVITE/200 OK SDP 中 `m=video` 是否被改写到 SIP3 公网 IP 和 RTP relay 端口；一路音视频 SIP 通话会占用 4 个 RTP relay 端口，端口池不足会比纯音频更早暴露。
- 浏览器 WebRTC 视频不等同于传统 SIP `m=video` relay；`webrtc_gateway.rs` 当前主要是音频桥接，WebRTC 视频需要单独设计 codec/track/SDP 处理。
- “会议无声音”重点查 Linphone 是否提供 RTP/AVP PCMU/PCMA、UDP `10100-10199` 是否放行、参与者 RTP peer 是否已学习。
- “语音信箱不接听/MWI 不亮灯”重点查目标信箱是否启用、`SUBSCRIBE Event: message-summary` 是否到达、请求源是否匹配 `sip_registrations.source_ip/source_port`。
- 不要把 voicemail `*97` 文档写成完整 IVR：v1.3.0 只提供基础访问/提示音准备和录音停止，完整播放导航仍是后续功能。

## 4. 发布与验证清单
- 本地：优先执行仓库根目录 `pwsh ./scripts/local-ci.ps1`（覆盖 backend `cargo fmt --check/build/test/clippy` + frontend `npm ci/build`）。
- 线上：`docker compose pull` + `docker compose up -d`、`/api/health`、关键日志无异常。
- 业务验收最少覆盖：
  1. MESSAGE 双向收发与入库；
  2. 双向呼叫（1001->1003、1003->1001）；
  3. 提前挂断、拒接、正常接通后双向语音；
  4. Linphone 双向视频通话，确认双方 SDP 的 `m=audio` 与 `m=video` 都指向 SIP3 relay 端口。
  5. Linphone 拨打默认会议室 `900000000`，确认两方以上可听到混音，`*6` 可切换静音。
  6. 离线/无人接听进入语音信箱，确认 WAV 入库、管理端可下载、MWI 数量更新。
  7. 发布 `v1.3.0` 前确认 `CHANGELOG.md`、`README.md`、`docs/deployment.md`、`docs/architecture.md`、`.github/copilot-instructions.md` 都同步更新。
