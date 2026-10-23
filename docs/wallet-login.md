# EOA 钱包登录（NAS 首期）

已实现 NAS 本地认证；仅支持 EOA，使用 SIWE（ERC-4361）及 EIP-191 personal_sign。
登录认证本身不调用 RPC、不发送交易、不产生 gas。Web 登录后另行查询或由用户确认登记到注册合约，见[首次绑定 NAS](nas-registration.md)。合约钱包登录及服务授权尚未实现；已接入的内容 API 见[发帖说明](posting.md)。
登录证明钱包控制权，不授予链上权限，也不能作为其他 NAS 或中继的登录凭证。

## 启动与配置

```sh
cargo run -p red-leaves
```

节点从仓库根目录 `.env.profile` 加载配置，模板见根目录 `.env.example`。默认 API 为 `127.0.0.1:3000`，SQLite 文件为工作目录下 `red-leaves.sqlite`。
首次启动使用当前完整 schema（version 6）创建认证和社交表，不再迁移旧数据库。已有库必须与当前版本一致，否则拒绝启动；旧开发数据可在停止 NAS 后删除数据库及其 WAL／SHM 文件和内容目录，再启动创建新库，步骤见[存储初始化说明](posting.md#存储初始化与版本)。

| 环境变量 | 默认值 | 用途 |
| --- | --- | --- |
| `RED_LEAVES_LISTEN` | `127.0.0.1:3000` | API 监听地址 |
| `RED_LEAVES_DATABASE_URL` | `sqlite://red-leaves.sqlite` | NAS SQLite URL，父目录须存在 |
| `RED_LEAVES_AUTH_URI` | `http://127.0.0.1:5173` | 可信登录站点 origin，无路径或查询参数 |

域名取自服务端配置，不信任客户端 Host 或转发头。配置只允许 HTTPS，回环 HTTP 用于本地开发。
Web 在 `web/` 执行 `npm ci`、`npm run dev`，默认直接请求 `http://127.0.0.1:3000/api`，不使用 Vite 代理。
可在 `web/.env.profile` 设置 `VITE_API_BASE_URL` 更改 API 地址（见 `.env.example`），修改后重启 Vite；生产构建时需设置实际 API 的 HTTPS 地址。
生产部署的 Web 与 API 均应使用 HTTPS，使用同源反向代理或同站点的子域名；当前 Cookie 为 SameSite=Lax，不支持不同站点之间的认证 Cookie。开发时 Web 与 API 应统一使用 127.0.0.1，或统一使用 localhost，不能混用。认证 URI 配置为可信 Web 登录站点 origin，与 API 的监听地址分开配置。
后端不自行终止 TLS，不能将明文 API 直接暴露到公网。代理访问日志不得记录请求正文、Authorization、Cookie 或 Set-Cookie。
App 使用该 NAS 配置的同一登录作用域，并通过 HTTPS 连接 API；App 的可信 NAS 发现与身份验证仍需后续接入。

## Web 与 App 的认证方式

Web 使用 NAS 下发的 `red_leaves_session` HttpOnly Cookie，浏览器通过 `credentials: "include"` 自动携带，前端脚本不读取令牌，也不将令牌保存到 localStorage 或 sessionStorage。Cookie 不设 Domain，限定 API 主机及 `Path=/api`，设置 `SameSite=Lax`、`Max-Age=3600`；可信登录 URI 为 HTTPS 时设置 `Secure`，仅回环开发允许 HTTP。

Web 所有 API 请求附带 `X-Red-Leaves-Client: web`，该请求头也用于选择 Cookie 登录响应。后端仅允许 `RED_LEAVES_AUTH_URI` 配置的 origin 通过带凭据的 CORS，允许 GET、POST 及 Content-Type、Authorization、X-Red-Leaves-Client 请求头，处理 OPTIONS 预检。Cookie 请求必须带上述自定义头，并拒绝与配置不符的 Origin；同源 GET 可不带 Origin。自定义头触发跨域预检，与来源校验及 SameSite 一起防止浏览器跨站请求伪造。Origin 本身不是身份凭证，仍须验证会话和 SIWE 签名。

刷新页面后，Web 先请求 `/api/auth/me`，再通过钱包的 `eth_accounts`、`eth_chainId` 静默核对地址和网络，不重新申请连接或签名。会话过期显示登录入口；钱包账户或网络不符时撤销当前会话。NAS 暂不可达或钱包尚未加载时保留 Cookie，并提供重试入口。退出时服务端撤销会话并以相同属性清除 Cookie；退出请求失败会提示撤销未完成，已有 Cookie 到期前仍可能恢复。

Web 使用 [EIP-6963](https://eips.ethereum.org/EIPS/eip-6963) 发现多个浏览器钱包，首次明确选择后，只将公开的钱包提供者标识（rdns）保存至 localStorage 的 `red-leaves:wallet-provider`，不保存令牌、私钥或账户凭据。刷新时优先匹配该钱包，扩展声明顺序或临时 UUID 改变不会触发重新选择；暂未加载时短暂等待并允许重试，不自动换用其他钱包。登录、发帖签名、资料交易和账户／网络变更监听均使用选定的 provider。退出保留钱包偏好；在登录页使用 `Choose another wallet` 可主动更换，成功登录后覆盖偏好。未记录偏好且发现多个钱包时显示选择入口；若浏览器禁止 localStorage，则只能在当前页面保持选择。旧钱包在没有 EIP-6963 声明时按单个注入提供者或可识别的品牌标志回退，多钱包可靠区分以 EIP-6963 为准。

原生 App 的 Bearer 接口保留：不带 Web 自定义头时，验证接口返回令牌，后续使用 `Authorization: Bearer <token>`；无 Origin 的原生请求仍可访问。若同时提供 Authorization 和 Cookie，优先检查 Authorization，错误时不回退 Cookie。原生 App 尚未实现，后续令牌应存系统安全存储。

当前无自动续期；会话有效期 1 小时，刷新不会延长有效期。退出只撤销当前会话，不撤销同钱包的其他设备会话。HttpOnly 限制脚本读取 Cookie，但仍需防范 XSS 代发请求，不能记录或传播会话令牌。

## API v1

请求与响应使用 JSON，时间字段为 Unix 秒；所有认证响应均带 `Cache-Control: no-store`。

1. `POST /api/auth/challenge`，请求 `{ "address": "0x…" }`。
   返回 `challenge_id`、`verifier`、`message`、`expires_at`。
   挑战使用 32 字节安全随机 nonce，5 分钟有效；verifier 为另一个独立随机秘密，数据库只存哈希。
   客户端只将完整 message 交给钱包签名，verifier 不需要签名，也不得放 URL、日志或公开消息中。
2. `POST /api/auth/verify`，请求 `challenge_id`、`verifier`、`signature`（0x 前缀的 65 字节签名）。
   NAS 从 SQLite 读取原始消息，验证绑定、有效期、配置域名、URI、固定链 ID `84532` 和 EOA 签名。
   客户端不能替换签名消息。Web 成功响应通过 Set-Cookie 写入令牌，JSON 仅返回 `address`（小写）、`chain_id`、`expires_at`；原生 Bearer 模式 JSON 另外返回 `token`。
   一次性消费挑战和创建会话在单个事务内提交；并发重放只能成功一次，插入失败回滚消费。
3. `GET /api/auth/me`，携带 Cookie 或 Bearer；返回 `address`、`chain_id`、`expires_at`。
4. `POST /api/auth/logout`，携带 Cookie 或 Bearer；删除当前会话，返回 204。Web 退出同时清除 Cookie，无 Cookie 或重复退出也返回 204；原生合法格式的已撤销令牌重复退出仍返回 204。

会话 token 为独立的 32 字节随机数，SQLite 只存 SHA-256 哈希。登录签名不保存。
错误：400 地址格式错误，401 认证无效或过期，403 浏览器来源或自定义头校验失败，413 请求超过 4 KiB，422 JSON 字段错误，
429 超出资源限制，408 请求超时，503 存储暂不可用，500 内部认证错误。
网络请求超时或响应丢失时，客户端可能无法取得已签发令牌；该会话到期自动失效。

## 资源限制与后续边界

每实例全局最多 240 次认证请求／分钟、16 个并发请求、10 秒请求超时；
每地址最多 10 个有效挑战、全局最多 1000 个有效挑战、10000 个有效会话。
每分钟清理过期记录。全局限制是首期保护，公网部署仍需网关按来源限流，避免单一来源耗尽全局配额。

SQLite 表见 [schema.sql](../crates/node/src/storage/schema.sql)。`nas` feature 启动上述 API；
`relay` 单独构建提供 P2P 中继，不提供认证 API，尚未实现 PostgreSQL 认证存储；同时启用时运行 NAS 实现。
登录认证本身不会创建用户服务密钥、查询绑定、向其他节点传播登录信息或解密私密内容；发帖模块另行执行身份绑定及访问权限检查。
受保护内容接口使用服务端会话解析及独立业务授权，不能信任请求中自行声明的钱包地址。
客户端钱包账户／网络变化、断开或退出时清理本地身份并尝试撤销服务端会话；撤销失败会提示会话到期失效。

## 验证

```sh
cargo fmt --all --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cd web
npm test
npm run build
```

后端测试覆盖登录／查询／退出、错误签名与验证密钥、错误作用域、挑战和会话过期、
并发重放、事务回滚、SQLite 重开、输入大小、Cookie 登录／清除／过期、来源校验、重复 Cookie 拒绝及资源限制。
前端使用模拟钱包与 API 测试消息编码、拒签、错误网络、账户切换及验证过程中退出。
真实钱包扩展端到端测试需另行执行。

标准参考：[ERC-4361](https://eips.ethereum.org/EIPS/eip-4361)、[ERC-191](https://eips.ethereum.org/EIPS/eip-191)。
