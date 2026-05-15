# SIP3 部署与排障经验总结（2026-05）

## 1. 本次问题根因
- 现象：Linphone 与网页可拨通，但接听后立即断开。
- 根因：`Linphone -> 网页` 方向原逻辑未进入 WebRTC 反向桥接分支，普通 SIP SDP 被按纯 RTP relay 处理，导致接听后协商链路不完整。

## 2. 修复要点
- 在 `proxy.rs` 增加“普通 SIP SDP + WebSocket 被叫”判定，触发反向 WebRTC 会话创建。
- 在 `webrtc_gateway.rs` 增加 SIP 主叫方向会话流（网关先发 offer，200 OK 时应用 browser answer）。
- 在 `handler.rs` 的 `relay_response()` 中按会话方向处理 200 INVITE SDP，避免混用正向/反向逻辑。
- 增加测试 `test_websocket_callee_with_plain_sdp_requires_reverse_bridge` 防回归。

## 3. 线上排障经验
- 先用 **Call-ID** 串联 CDR、SIP 日志与端侧日志，再判断谁先发 BYE，避免误改。
- “接听即断开”优先检查：`INVITE/200/ACK/BYE` 完整性与 SDP 方向匹配，而不是先怀疑网络。
- WebRTC 网关中 SIP 对端地址学习不应只依赖第一次包，源地址变化时应允许更新。

## 4. 发布与验证建议
- 变更前先跑后端 `clippy -D warnings` 与 `cargo test`。
- 部署后至少验证：容器健康、`/api/health`、关键日志无异常。
- 双向呼叫（网页->Linphone / Linphone->网页）都需回归验证，避免只测单方向。
