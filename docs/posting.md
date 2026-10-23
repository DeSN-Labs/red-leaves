# NAS 发帖与接收范围

日期：2026-10-23

已实现文本／图文／视频发帖：用户签名并确认发布，NAS 保存内容、管理关注关系与接收偏好，并按受众权限提供正文和附件。帖子不设置价格，不需要链上登记、购买或付款解锁；中继及托管服务的费用属于独立的后续设计。SQLite 保存元数据和任务，文件系统保存公开对象或密文，客户端不排序时间线。

已接入同 NAS／跨 NAS 范围投递、Relay v2 中继、离线重试、实时收件箱和远程媒体按需读取，配置与边界见[按范围推送](scoped-delivery.md)。公开广播面向已连接网络，不表示已经送达全部节点。设备委托签名、群聊、自动全网发现与真实资金结算仍待实现。

## 配置和使用

实际配置写入 NAS 根目录 `.env.profile`，模板见[根配置](../.env.example)：

```dotenv
RED_LEAVES_RPC_URL=https://sepolia.base.org
RED_LEAVES_USER_REGISTRY_ADDRESS=<existing UserRegistry address>
RED_LEAVES_CONTENT_PATH=runtime/nas/content
RED_LEAVES_MASTER_KEY_PATH=runtime/nas/master.key
RED_LEAVES_USER_KEY_PATH=runtime/nas/user-keys
```

RPC 必须属于 Base Sepolia（84532），外部地址使用 HTTPS；本地测试允许 loopback HTTP。RPC 查询失败暂停依赖最新身份绑定的操作，不把缓存当作最新绑定证明。Web 使用 `web/.env.profile` 中的 API 和注册合约地址；两个工程应配置相同 UserRegistry。旧 `RED_LEAVES_POST_ACCESS_ADDRESS` 配置已不再读取，可以移除。

1. 重启最新 NAS：`cargo run -p red-leaves`；启动 Web：`cd web && npm run dev`。
2. 钱包登录后，首次登记使用 NAS 生成的 X25519 公钥。旧用户若登记的是浏览器密钥，点击左下角钱包进入个人资料页，在 `NAS encryption key` 中查看 NAS 公钥，并主动确认链上更新交易。旧浏览器私钥不会导出，也不会自动迁移旧密文。
3. 在“我的发布”选择文字、图片或视频，填写标题及可选配文与传播范围。纯文字帖必须有正文；图片或视频帖可省略正文。正文最多 16 KiB UTF-8，标题最多 256 字节，非广播范围最多 32 个不同接收钱包，不包含作者本人。
4. 确认内容、附件及范围，勾选发布确认，再由钱包签署明确的 `SocialAction`。签名不会转账。NAS 验证签名和一次性挑战后，在同一事务中保存帖子并建立投递任务，返回 `saved_pending_network`；不需要第二次合约登记或激活。
5. “我的关注”支持独立签名关注／取消关注并同步到目标 NAS。“关注动态”可组合接收全网广播、我关注的人、关注我的人、明确发给我的帖子。默认接收我关注的人的帖子和明确发给我的帖子，不默认接收所有广播。

## 内容形式、受众与媒体

支持标题＋文字、标题＋文字＋图片、标题＋图片、标题＋视频、标题＋文字＋视频。标题必填；图片最多 9 张，每张 10 MiB，支持 JPEG／PNG／WebP／GIF；视频仅 1 个，最大 64 MiB，支持 MP4／WebM；图片和视频不能混发。第一张图片作为封面。只做 MIME、大小及基本文件头检查，不转码、不保证浏览器支持所有编码。

列表使用封面卡片，点击打开图片轮播／视频与文字的详情；后端按保存时间和 ID 倒序分页返回，每页最多 50 条，前端保持顺序。媒体进入可见区域后才请求，视频首版完整下载到有界 Blob 后播放，不支持分段流播、服务端缩略图或断点续传。

传播范围为 `broadcast`（全网广播）、`following`（我关注的人）、`followers`（关注我的人）、`friends`（互相关注）、`directed`（指定的人）。指定人可勾选好友／关注列表并补充钱包地址，合并去重后签名。关系范围由当前 NAS 保存及同步的真实关注关系解析，服务端准备签名时核对精确名单；名单不符返回 409，须刷新后重新确认。签名后名单固定，后续关系变化不会扩大旧帖授权。除 broadcast 外全部加密定向存储与投递，接收偏好不会扩大访问权限；黑名单用户的内容不得被动投递。

上传采用 `POST /api/media`，Body 是原始文件，Content-Type 必须是允许的 MIME，使用 SIWE 会话认证（Web Cookie／原生 Bearer）。上传仅产生私有草稿，不建立投递任务。`GET /api/media` 可恢复未引用上传；`POST /api/media/{id}/discard` 仅删除自己的未引用文件，已被帖子引用的文件不能删除。发布的 `media` 包含上传 ID、MIME、原始字节大小与 SHA-256 multihash 摘要，均纳入规范 CBOR 和签名；准备及提交检查文件归属和元数据。

每个文件以独立 AES-256-GCM 密钥和 nonce 加密落盘，包括公开帖的附件；密钥通过 HPKE 封装给作者 NAS，AAD 为 `red-leaves/media/1/{lowercase-owner}/{upload-id}`。Unix 文件权限 600。读取验证 AEAD 和签名引用的长度、摘要；落盘加密不改变广播附件的公开属性。

草稿预览 `/api/media/{id}` 仅作者可读。其他用户通过 `/api/posts/{post-id}/media/{id}` 读取，NAS 核对发布状态、签名引用及受众，不查询付款状态。跨 NAS 源节点还核对请求钱包绑定的 NAS，并为接收者封装文件密钥，按有界密文块传输。视频 URL 使用认证 Blob，页面退出后撤销对象 URL，不把 Bearer 放入查询参数。单钱包媒体总配额 512 MiB、未引用上传最多 32 个，Web 同时最多两个媒体下载。

## 签名、加密与持久化

`social::Intent` 包含 scope、actor、NAS PeerId、nonce、签发／过期时间及 `Publish`／`Follow` 动作，使用严格确定性 CBOR。新 `Publish.post` 只包含 `title`、`body`、`distribution`、`recipients` 及可选 `media`，不含价格字段；旧请求即使携带 `price_wei: "0"` 也会被拒绝。空 `media` 字段省略。

EIP-712 域沿用 `RedLeaves`／`1`、链 ID、UserRegistry 地址，主类型为 `SocialAction(bytes32 payloadHash)`，内容 ID 域仍为 `red-leaves/social/1\0`。完整载荷参与哈希，移除字段后的新帖子具有不同的签名与 ID；不复用 SIWE 或旧 `PublicEvent` 签名。准备挑战有效期 15 分钟，每用户最多 16 个待签名动作。

Web 在签名前独立重建 CBOR，核对显示内容、附件、受众、钱包、NAS 和链范围；NAS 提交时验证 EOA 签名、会话持有人、用途及挑战，事务性消费一次性挑战并保存数据和任务。仅准备签名不会发布。历史帖子读取复核原始签名及内容 ID，不重新套用提交时的有效期规则。

广播帖子保存公开签名载荷；其他范围的正文和标题使用 AES-256-GCM 加密，内容密钥通过 HPKE 分别封装给作者及目标接收者。AAD 为 `red-leaves/post/1/{id}`，密钥封装再绑定接收钱包；没有额外的付费加密层。HPKE 使用 RFC 9180 Base mode：X25519-HKDF-SHA256、HKDF-SHA256、ChaCha20Poly1305，应用公钥为原始 32 字节 X25519。

发布前检查作者最新链上 PeerId 和公钥确实对应本 NAS；创建定向封装时重新读取接收方公钥，本 NAS 内的接收者同样必须登记本 NAS 持有的公钥。完整密钥轮换、服务 grant 和撤销传播仍待实现。

应用私钥以 AES-256-GCM 加密后按用户保存，主密钥单独存储；密钥文件和 SQLite 及 WAL／SHM 权限收紧到 600。须备份主密钥、每用户密钥、节点身份、数据库及内容目录。已有用户密钥而主密钥丢失时不自动生成替代，损坏文件不覆盖；不承诺对 NAS 管理员隐藏内容，尚无自动备份／恢复界面。

对象文件先可靠写入，再提交数据库引用和任务；失败或并发可能留下未引用文件，后续需维护性清理，不把它们当作有效帖子。已存副本不会因本地取消关注而远程删除。

## 存储初始化与版本

SQLite 使用一份完整的 [schema.sql](../crates/node/src/storage/schema.sql) 初始化当前 version 6 数据库，不保留历史迁移、归档表或旧数据读取分支。空库首次启动自动建表，当前版本正常重开；其他版本拒绝启动，不将旧表当作新格式使用。

旧开发数据可直接删除，无需迁移或导入。先停止使用该数据库的 NAS，再删除 `RED_LEAVES_DATABASE_URL` 指定的 SQLite 文件及对应的 `-wal`、`-shm` 文件，同时清理 `RED_LEAVES_CONTENT_PATH` 内容目录；下一次启动创建新库。默认对应工作目录下的 `red-leaves.sqlite` 和 `runtime/nas/content`。删除数据库后需重新登录，原帖子、关注、偏好和投递状态不恢复。节点身份、主密钥和用户密钥不在内容目录中，不属于此次数据重建范围。

帖子对象只读取当前 version 2 格式；只验证当前无价格字段的签名，不兼容旧免费帖或付费帖。详情的 `intent` 可直接用于重建规范载荷验签，不再为旧签名额外返回原始 `payload`。

跨节点使用 delivery v2（`/red-leaves/delivery/2.0.0`），公开主题也使用 v2。通知和正文响应不包含价格、交易合约或付费密钥，只接受当前响应结构及 `red-leaves/delivery/2/{post-id}/{lowercase-recipient}` 加密上下文。所有参与投递的 NAS 需同步升级，旧版节点无法协商新应用协议，不降级公开传输。底层 libp2p Relay v2 是独立协议。

## API

以下接口均需要 SIWE 会话（Web 使用 HttpOnly Cookie，原生客户端可使用 Bearer），认证与来源保护见[钱包登录](wallet-login.md)：

| 方法和路径 | 功能 |
| --- | --- |
| `GET/POST /api/media` | 列出自己的未引用文件／上传原始媒体字节 |
| `GET /api/media/{id}` | 作者预览私有上传 |
| `POST /api/media/{id}/discard` | 作者删除未引用上传 |
| `GET /api/posts/{id}/media/{media-id}` | 校验签名引用及受众后读取媒体 |
| `GET /api/content/key` | 生成或恢复当前用户 NAS 密钥，只返回公钥、算法和保管方式 |
| `POST /api/social/prepare` | 准备明确用户动作及规范签名载荷，不发布 |
| `POST /api/social/commit` | 验证签名并消费挑战，保存并排队投递帖子或保存关注变更 |
| `GET/POST /api/social/preferences` | 查询／修改四类接收来源，按 OR 组合 |
| `GET /api/social/relationships` | 本 NAS 的关注和关注者 |
| `GET /api/posts?mine=true&offset=0` | 我的帖子及投递进度；省略 mine 则按接收偏好筛选 |
| `GET /api/posts/{id}` | 验证受众，返回 `id`、`intent` 及 `signature` |
| `GET /api/posts/{id}/delivery` | 作者查询逐目标投递状态 |

发布提交返回 `id`、`author` 和 `status: "saved_pending_network"`；关注提交返回 `status: "saved"`。帖子列表及详情不返回价格、市场合约、`locked` 或付费密钥。旧 `/listing`、`/activate`、`/purchase-info`、`/unlock` 路由已删除，返回 404。

JSON 内容请求最大 128 KiB；独立媒体路由最大 64 MiB，图片再限制 10 MiB。共用 NAS 请求并发及速率限制；内容操作总超时为 60 秒，接收者链查询最多 4 个并发。`401` 为会话失效，`404` 合并不存在／无权限，`409` 为绑定／密钥／受众不匹配，`422` 为 JSON 结构不符（含旧价格字段），`503` 为链或存储不可用。错误不回显私钥、令牌或带凭据的 RPC URL。

## 验证

```sh
cargo fmt --check
cargo check --workspace --all-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build -p red-leaves --locked
cd contracts && forge test
cd ../web && npm test && npm run build
# Local integration; build the NAS and contracts first. No real chain writes.
node --experimental-strip-types tests/publishing.anvil.mjs
node --experimental-strip-types tests/delivery.anvil.mjs
```

测试覆盖签名确认、幂等提交、拒绝旧价格字段、删除旧付款接口、受众和附件隔离、密文完整性、拒绝旧对象格式、当前 schema 初始化及重开、拒绝不匹配的数据库版本。独立 Anvil／NAS 场景验证客户端 CBOR 与 EIP-712、直接发布、公开及定向媒体，以及多节点中继、重试和缓存读取；这些检查不替代真实浏览器钱包扩展或测试网端到端验收。
