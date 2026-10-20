# EOA 钱包登录（NAS 首期）

已实现 NAS 本地认证；仅支持 EOA，使用 SIWE（ERC-4361）及 EIP-191 personal_sign。
登录认证本身不调用 RPC、不发送交易、不产生 gas。Web 登录后另行查询或由用户确认登记到注册合约，见[首次绑定 NAS](nas-registration.md)。合约钱包登录、服务授权及内容 API 尚未实现。
登录证明钱包控制权，不授予链上权限，也不能作为其他 NAS 或中继的登录凭证。

## 启动与配置

```sh
cargo run -p red-leaves
```

节点从仓库根目录 `.env.profile` 加载配置，模板见根目录 `.env.example`。默认 API 为 `127.0.0.1:3000`，SQLite 文件为工作目录下 `red-leaves.sqlite`。
首次启动自动创建认证表（schema version 1）；数据库版本高于程序支持的版本时拒绝启动。

| 环境变量 | 默认值 | 用途 |
| --- | --- | --- |
| `RED_LEAVES_LISTEN` | `127.0.0.1:3000` | API 监听地址 |
| `RED_LEAVES_DATABASE_URL` | `sqlite://red-leaves.sqlite` | NAS SQLite URL，父目录须存在 |
| `RED_LEAVES_AUTH_URI` | `http://127.0.0.1:5173` | 可信登录站点 origin，无路径或查询参数 |

域名取自服务端配置，不信任客户端 Host 或转发头。配置只允许 HTTPS，回环 HTTP 用于本地开发。
Web 在 `web/` 执行 `npm ci`、`npm run dev`，默认直接请求 `http://127.0.0.1:3000/api`，不使用 Vite 代理。
可在 `web/.env.profile` 设置 `VITE_API_BASE_URL` 更改 API 地址（见 `.env.example`），修改后重启 Vite；生产构建时需设置实际 API 的 HTTPS 地址。
生产部署的 Web 与 API 可以使用不同域名，两者均应使用 HTTPS；认证 URI 配置为可信 Web 登录站点地址，与 API 的监听地址分开配置。
后端不自行终止 TLS，不能将明文 API 直接暴露到公网。代理访问日志不得记录请求正文或 Authorization。
App 使用该 NAS 配置的同一登录作用域，并通过 HTTPS 连接 API；App 的可信 NAS 发现与身份验证仍需后续接入。

## Web 与 App 的认证方式

两者均使用 `Authorization: Bearer <token>`，不使用认证 Cookie，不依赖请求 Origin，
后端允许任意来源的 CORS，支持 GET、POST 及 Content-Type、Authorization 请求头，处理 OPTIONS 预检；不允许携带 Cookie 凭据。
Web 可直接跨域请求 API；没有 Origin 的原生 App 请求也能访问。跨域放行不会跳过 Bearer 验证或 SIWE 签名作用域校验。
Origin 不能证明客户端身份。浏览器不会自动附带本方案的 Bearer 令牌，因此没有 Cookie 自动携带导致的 CSRF 登录态问题。
SIWE 消息的 domain、URI 仍绑定具体服务，不能省略或换成客户端任意输入。

Web 令牌只保存在模块内存，刷新后需重新登录；未退出的旧会话在服务端到期失效。
原生 App 尚未实现，后续令牌应存系统安全存储。令牌具有完整会话权限，应避免 XSS、泄漏与日志记录。
当前无自动续期；会话有效期 1 小时。退出撤销当前会话，不撤销同钱包的其他设备会话。

## API v1

请求与响应使用 JSON，时间字段为 Unix 秒；所有认证响应均带 `Cache-Control: no-store`。

1. `POST /api/auth/challenge`，请求 `{ "address": "0x…" }`。
   返回 `challenge_id`、`verifier`、`message`、`expires_at`。
   挑战使用 32 字节安全随机 nonce，5 分钟有效；verifier 为另一个独立随机秘密，数据库只存哈希。
   客户端只将完整 message 交给钱包签名，verifier 不需要签名，也不得放 URL、日志或公开消息中。
2. `POST /api/auth/verify`，请求 `challenge_id`、`verifier`、`signature`（0x 前缀的 65 字节签名）。
   NAS 从 SQLite 读取原始消息，验证绑定、有效期、配置域名、URI、固定链 ID `84532` 和 EOA 签名。
   客户端不能替换签名消息。成功返回 `token`、`address`（小写）、`chain_id`、`expires_at`。
   一次性消费挑战和创建会话在单个事务内提交；并发重放只能成功一次，插入失败回滚消费。
3. `GET /api/auth/me`，携带 Bearer；返回 `address`、`chain_id`、`expires_at`。
4. `POST /api/auth/logout`，携带 Bearer；删除当前会话，返回 204。合法格式的已撤销令牌重复退出仍返回 204。

会话 token 为独立的 32 字节随机数，SQLite 只存 SHA-256 哈希。登录签名不保存。
错误：400 地址格式错误，401 认证无效或过期，413 请求超过 4 KiB，422 JSON 字段错误，
429 超出资源限制，408 请求超时，503 存储暂不可用，500 内部认证错误。
网络请求超时或响应丢失时，客户端可能无法取得已签发令牌；该会话到期自动失效。

## 资源限制与后续边界

每实例全局最多 240 次认证请求／分钟、16 个并发请求、10 秒请求超时；
每地址最多 10 个有效挑战、全局最多 1000 个有效挑战、10000 个有效会话。
每分钟清理过期记录。全局限制是首期保护，公网部署仍需网关按来源限流，避免单一来源耗尽全局配额。

SQLite 表见 `crates/node/migrations/0001_auth.sql`。`nas` feature 启动上述 API；
`relay` 单独构建仍是骨架，尚未实现 PostgreSQL 认证存储；同时启用时运行 NAS 实现。
不会创建用户服务密钥、查询绑定、向其他节点传播登录信息或进行私密内容解密。
未来受保护内容接口必须使用服务端会话解析及独立业务授权，不能信任请求中自行声明的钱包地址。
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
并发重放、事务回滚、SQLite 重开、输入大小、Cookie 无效及资源限制。
前端使用模拟钱包与 API 测试消息编码、拒签、错误网络、账户切换及验证过程中退出。
真实钱包扩展端到端测试需另行执行。

标准参考：[ERC-4361](https://eips.ethereum.org/EIPS/eip-4361)、[ERC-191](https://eips.ethereum.org/EIPS/eip-191)。
