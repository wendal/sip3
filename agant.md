# SIP3 部署与排障经验总结（v1.3.0）

## 1. 生产部署
- 使用 Docker + Nginx + Certbot，Web 页面走 `443`，浏览器 SIP 走 `WSS 5443`。
- 后端 API 与前端容器端口建议仅绑定 `127.0.0.1`，对外只暴露必要入口端口。
- 在国内网络环境下，建议提前配置镜像源（apt/cargo/npm）避免构建超时。

## 2. SIP 与浏览器互通关键点
- 仅有数据库注册信息不够，浏览器连接必须保留“在线可写”传输通道（WS/WSS 连接注册表）。
- 给浏览器终端转发 INVITE 时，需要优先走已建立的 WS/WSS 流，不应回退到 UDP 地址。
- 浏览器到浏览器通话场景要保留 WebRTC SDP；只在 WebRTC 到传统 SIP 终端时做必要改写。

## 3. 数据与后端稳定性
- MySQL `BIGINT UNSIGNED` 不要直接解码到 Rust `i64`，避免 SQLx 运行期失败。
- 已上线迁移文件不要改历史内容，新增迁移（如 `008_*`）做增量修复更安全。
- SIP 账号规则统一为 3-6 位纯数字分机（如 `1001/1002/1003`），前后端都要校验。
- 会议室分机固定为 9 位数字，避免与 3-6 位用户分机冲突；默认会议室可用于 Linphone 冒烟测试。
- 语音信箱表和迁移要同时放在 `migrations/` 与 `backend/migrations/`，因为后端会嵌入 SQLx 迁移。
- 语音信箱 WAV 存储键必须避免重复 Call-ID 覆盖；使用唯一 `storage_key` 并配合数据库唯一索引。

## 4. 前端可用性
- 拨号盘必须包含 `0`，否则默认数字分机体系无法完整输入。
- 呼叫中间态（拨号中/失败/已结束）要有明确 UI 状态机，失败后可恢复并可挂断。
- Firefox 出现 `The object can not be found here.` 常见于麦克风权限或媒体设备不可用，应给出可读错误提示。

## 5. 发布流程
- 发布前优先在仓库根目录执行 `pwsh ./scripts/local-ci.ps1`，确保与 CI 的 backend `fmt/build/test/clippy` 和 frontend `npm ci/build` 对齐。
- 使用语义化标签发布（如 `v1.3.0`），并同步创建 GitHub Release。
- 分支完成后及时合并、推送主干，并清理 feature 分支与 worktree，保持仓库整洁。
- v1.3.0 发布前必须验证四段媒体端口不重叠：通话 relay `10000-10099`、会议 `10100-10199`、语音信箱 `10200-10299`、WebRTC `20000-20099`。
- 生产部署路径为 `/opt/sip3` 时，优先 `git pull --ff-only` + `docker compose pull` + `docker compose up -d`，不要覆盖生产本地配置或密钥。
