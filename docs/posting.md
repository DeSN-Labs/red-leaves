# NAS 发帖、接收范围与付费解锁

日期：2026-10-21

当前已实现文本／图文／视频发帖的本地 NAS 闭环：用户签名并确认发布，保存帖子、管理关注关系与接收偏好、读取获授权内容，并通过测试网合约控制付费解锁。SQLite 保存元数据和任务，文件系统保存公开对象或密文，客户端不排序时间线。

已接入同 NAS／跨 NAS 范围投递、Relay v2 中继、离线重试、实时收件箱和远程媒体按需读取，配置与边界见[按范围推送](scoped-delivery.md)。公开广播面向已连接网络，不能解释为已经送达全部节点。设备委托签名、群聊、自动全网发现与真实资金结算仍待实现。

## 配置和使用

NAS 根目录 `.env.profile`，模板见 [根配置](../.env.example)：

```dotenv
RED_LEAVES_RPC_URL=https://sepolia.base.org
RED_LEAVES_USER_REGISTRY_ADDRESS=<existing UserRegistry address>
RED_LEAVES_POST_ACCESS_ADDRESS=<PostAccess address, required for paid posts>
RED_LEAVES_CONTENT_PATH=runtime/nas/content
RED_LEAVES_MASTER_KEY_PATH=runtime/nas/master.key
RED_LEAVES_USER_KEY_PATH=runtime/nas/user-keys
```

RPC 必须属于 Base Sepolia（84532），外部地址使用 HTTPS；本地测试允许 loopback HTTP。RPC 查询失败暂停依赖它的发布／解锁，不把缓存当作最新绑定或付款证明。Web 继续使用 `web/.env.profile` 中的 API 和注册合约地址；两个工程应配置相同 UserRegistry。

1. 重启最新 NAS：`cargo run -p red-leaves`；启动 Web：`cd web && npm run dev`。
2. 钱包登录后，首次登记使用 NAS 生成的 X25519 公钥。旧用户若登记的是浏览器密钥，点击左下角钱包进入个人资料页，在 `NAS encryption key` 中查看 NAS 公钥，并主动确认链上更新交易。旧浏览器私钥不会被导出，也不会自动迁移旧密文。
3. 在“我的发布”选择文字、图片或视频，填写标题及可选配文、免费／付费与传播范围。纯文字帖必须有正文；图片或视频帖可省略正文。价格单位为 Base Sepolia 测试 ETH，最多 18 位小数；正文最多 16 KiB UTF-8，标题最多 256 字节，非广播范围最多 32 个不同接收钱包，不包含作者本人。
4. 确认内容、价格及范围，勾选发布确认，再由钱包签署明确的 `SocialAction`。签名不会转账。NAS 验证签名和一次性挑战后持久化；只有 `publish` 动作才建立发布许可。
5. 付费帖先保存为 `awaiting_listing`，此时其他用户不能获取正文或通知。用户再用钱包确认 `PostAccess.registerPost` 的上链交易及 gas；NAS 核对合约记录后才激活并建立待投递任务。中断后用 `Complete paid listing` 恢复；已存在的匹配登记不重复发送登记交易。
6. “我的关注”支持签名关注／取消关注并同步到目标 NAS，操作独立签名。“关注动态”可组合接收全网广播、我关注的人、关注我的人、明确发给我的帖子。默认接收我关注的人的帖子和明确发给我的帖子，不默认收取所有广播。

## 内容形式、卡片与受众

支持五种组合：标题＋文字、标题＋文字＋图片、标题＋图片、标题＋视频、标题＋文字＋视频。标题必填；图片最多 9 张，每张 10 MiB，支持 JPEG／PNG／WebP／GIF；视频仅 1 个，最大 64 MiB，支持 MP4／WebM；图片和视频不能混发。第一张图片作为封面。只做 MIME、大小及基本文件头检查，不转码、不保证浏览器支持所有编码。

列表使用封面卡片，点击打开图片轮播／视频与文字的详情；后端返回时间倒序，前端保持顺序，不做标签分类。付费内容解锁前不展示真实标题、封面或媒体引用。媒体进入可见区域后才请求，视频首版完整下载到有界 Blob 后播放，不支持分段流播、服务端缩略图或断点续传。

传播范围是 `broadcast`（全网广播）、`following`（我关注的人）、`followers`（关注我的人）、`friends`（互相关注）、`directed`（指定的人）。指定人可勾选好友／关注列表并补充钱包地址，合并去重后签名。其余三种关系范围由当前 NAS 保存及同步的真实关注关系解析，服务端准备签名时核对精确名单；关系变动导致名单不符时返回 409，必须刷新后重新确认。受众为空或超过 32 人时应使用指定人缩小范围。签名后名单固定，后续关注变化不会扩大旧帖授权；只有 broadcast 使用公开传播，其余范围全部走加密定向存储与待投递队列。

昵称登记、NAS 绑定和公钥管理统一在左下角钱包按钮进入的个人资料页；登录不自动登记、不自动更新链上数据。

## 媒体上传与访问边界

上传采用 `POST /api/media`，Body 是原始文件，Content-Type 必须是允许的 MIME；使用与发帖相同的 SIWE Bearer 认证。文件在内容目录独立保存，SQLite 保存归属、长度、摘要、加密密钥封装和帖子引用。单钱包媒体总配额 512 MiB、未引用上传最多 32 个，同时最多两个媒体请求；有界文件大小与既有请求速率／超时共同限制资源消耗。

上传成功只产生私有草稿文件，不产生投递任务。`GET /api/media` 可恢复之前的未引用上传；`POST /api/media/{id}/discard` 仅能删除自己的未引用文件，待付费登记的文件同样不能删除。发布时 `media` 引用包含随机上传 ID、MIME、原始字节大小与 SHA-256 multihash 摘要，并被纳入规范 CBOR 与 EIP-712 签名；准备／提交均检查文件归属及元数据。用户不能借其他人的上传发布或替换已签名附件。

每个文件以独立 AES-256-GCM 随机密钥和 nonce 加密落盘（包括尚未发布和免费公开文件）；密钥通过 HPKE 封装给作者 NAS，AAD 为 `red-leaves/media/1/{lowercase-owner}/{upload-id}`。Unix 文件权限 600。读取验证 AEAD 及签名引用中的长度和摘要；公开免费帖的附件经 API 读取后是公开内容，落盘加密不改变其公开属性。

草稿预览 `/api/media/{id}` 仅作者可读；其他用户必须走 `/api/posts/{post-id}/media/{id}`，NAS 核对已发布状态、签名附件引用、受众，以及付费帖的链上付款状态。付款前返回 402，不交付原图、视频或文件密钥。付费附件目前由可信作者 NAS 解密后通过认证 HTTP 交付，其文件密钥独立于正文的合约付费密钥；跨 NAS 读取时，源 NAS 在校验受众／付款后，向接收者 NAS 封装该文件密钥并提供有界密文块，见[推送说明](scoped-delivery.md)。原型 P2P 分页清单和断点续传仍待接入。视频 URL 使用需认证的 Blob，页面退出后撤销对象 URL，不把 Bearer 放入查询参数。

## 付费合约部署

使用现有 `contracts/.env.profile` 中的部署 RPC 和专用测试私钥：

```sh
cd contracts
bash script/deploy.sh --post-access
# Review the simulation before broadcasting with your funded testnet deployer.
bash script/deploy.sh --post-access --broadcast
```

将返回的 PostAccess 地址填入根 `.env.profile` 的 `RED_LEAVES_POST_ACCESS_ADDRESS`，重启 NAS。此操作部署独立合约，不改变 UserRegistry 的四项用户资料。实现阶段只执行了本地链测试，未代用户广播该合约到 Base Sepolia。

买家点击 `Pay and unlock`，钱包向该合约付款，合约将金额直接转给作者，无平台抽成。同一作者／帖子／买家不能重复购买，价格和密钥登记不可覆盖；付款金额不匹配或作者收款失败时整笔交易回滚。页面可用 `Check existing payment` 重新验证已有付款。

## 明文、密文与密钥

| 类型 | 存储及未来投递的内容 | 解锁条件 |
| --- | --- | --- |
| 免费广播 | 公开的规范签名载荷，不做内容加密 | 已发布，允许公开读取 |
| 免费定向 | AES-256-GCM 密文，每个接收者有独立 HPKE 内容密钥封装 | 是目标接收者；私钥由其 NAS 保管 |
| 付费广播 | 公开作者、价格和帖子标识，标题与正文使用独立付费密钥加密 | 作者，或已确认付款的买家 |
| 付费定向 | 付费加密层外再加定向传输层 | 必须既在定向名单中，又已确认付款；作者可读取 |

付费广播不广播完整明文，这是“付费内容额外加密”对免费公开广播规则的补充。定向帖和所有付费帖的标题也保存在密文内，SQLite／未解锁列表只显示 `Encrypted post`；仅免费广播标题明文公开。价格、作者、帖子标识及买家付款记录属于公开元数据。接收范围只决定被动接收，不授予定向访问权，也不绕过付款。

正文对称算法为 AES-256-GCM，每个帖子／加密层独立产生随机 256 位内容密钥和 96 位 nonce。密钥封装采用 RFC 9180 HPKE Base mode：X25519-HKDF-SHA256、HKDF-SHA256、ChaCha20Poly1305。当前应用公钥格式明确约定为原始 32 字节 X25519；不尝试其他算法，也不以旧浏览器缺钥为由自动覆盖链上登记。

加密上下文绑定帖子 ID：定向层 AAD 为 `red-leaves/post/1/{id}`；其密钥封装再绑定接收钱包。付费层 AAD 额外包含 `/paid`；交付买家的封装额外绑定买家钱包。HPKE info 使用独立版本化用途域。帖子 ID 绑定网络、作者、内容、价格、受众、nonce、NAS 和动作。内层加密结构使用规范 CBOR，避免 JSON 字节数组造成多层封装膨胀。

**合约中的任何字节都公开可读取。** 合约保存的付费内容密钥封装加密给作者 NAS，不能提前加密给尚未付款的买家。NAS 通过已接受区块的 `purchased(author, postId, buyer)` 验证付款后，读取链上最新买家公钥，重新封装付费密钥给买家；HTTP 原型同时通过认证连接返回已解密正文。普通读帖接口对付费买家始终先返回锁定预览，必须调用解锁接口。

前端等待三次确认，NAS 使用当前高度减 2 的状态检查登记及付款；这不是链最终性保证。已交付的明文或密钥无法收回。该方案依赖作者 NAS 在线且诚实交付，合约不会保守秘密或自动重新加密，也不提供无信任交付、自动退款或内容正确性证明。当前只用于测试网验证，真实资金规则另行设计。

## 签名、权限与持久化

`social::Intent` 包含 scope、actor、NAS PeerId、nonce、签发／过期时间及 `Publish`／`Follow` 动作，采用严格确定性 CBOR。EIP-712 域沿用 `RedLeaves`／`1`、链 ID、UserRegistry 地址，主类型独立为 `SocialAction(bytes32 payloadHash)`；不复用 SIWE 登录签名或旧 `PublicEvent` 签名作为发布许可。准备挑战有效期 15 分钟，每用户最多 16 个待签名动作。

Web 在签名前独立重建 CBOR，并核对显示内容、受众、价格、钱包、NAS 和链范围；NAS 提交时验证 EOA 签名、会话持有人、用途及挑战，在同一 SQLite 事务中消费挑战、记录元数据和建立任务。仅签名准备不会写帖子或传播内容。已接受历史帖读取时复核内容 ID 和签名，不重新套用“提交挑战尚未过期”的规则。

发布前验证作者链上 PeerId 和公钥确实对应本 NAS 持有的应用密钥；每次建立定向封装重新读取接收方公钥，不使用旧缓存回退。本 NAS 内的接收者同样必须登记本 NAS 持有的公钥。当前应用传输校验最新链上 NAS 绑定和完整用户签名；外部算法协商、完整轮换与服务 grant 授权仍待实现。

应用私钥以 AES-256-GCM 加密后按用户保存，主密钥单独保存在文件中；新密钥文件 Unix 权限为 600，并同步文件／目录；SQLite 数据库及已有 WAL／SHM 文件也收紧到 600。必须备份主密钥、每用户密钥、节点身份、SQLite 及内容目录。已有用户密钥但主密钥丢失时拒绝自动生成替代；损坏文件不覆盖。主密钥仍能被 NAS 管理员读取，此机制不承诺对管理员隐藏内容，尚无自动备份／恢复界面。

SQLite v4 自动迁移既有 v2／v3 数据，保留旧帖子；空 `media` 字段省略，因此旧文本签名和 ID 不变。新增媒体字段与三类关系范围是本地 `social::Intent` 的增量扩展，旧实现遇到新载荷应拒绝，不可降级为广播；当前应用推送使用独立的 delivery v1 流协议，原型 HELLO／Frame 能力协商仍待接入。

对象文件先可靠写入，再提交数据库引用和任务；失败或并发可能留下未引用文件，后续需要维护性清理，不把它们当作有效帖子。当前并无完整用户接纳、磁盘配额和分布式去重服务。时间线由后端按保存时间和 ID 倒序、每页最多 50 条返回；前端按返回顺序展示。已存副本不会因为本地取消关注被远程删除。

## API

以下接口均需要 SIWE Bearer 会话，不使用 Cookie：

| 方法和路径 | 功能 |
| --- | --- |
| `GET/POST /api/media` | 列出自己的未引用文件／上传原始媒体字节 |
| `GET /api/media/{id}` | 作者预览私有上传 |
| `POST /api/media/{id}/discard` | 作者删除未引用上传 |
| `GET /api/posts/{id}/media/{media-id}` | 校验签名引用、受众及付款后读取媒体 |
| `GET /api/content/key` | 生成或恢复当前用户 NAS 密钥，只返回公钥、算法和保管方式 |
| `POST /api/social/prepare` | 准备明确用户动作及规范签名载荷，不发布 |
| `POST /api/social/commit` | 验证签名并消费挑战，保存帖子或关注变更 |
| `GET/POST /api/social/preferences` | 查询／修改四类接收来源，按 OR 组合 |
| `GET /api/social/relationships` | 本 NAS 的关注和关注者 |
| `GET /api/posts?mine=true&offset=0` | 我的帖子及待完成登记；省略 mine 则按接收偏好筛选 |
| `GET /api/posts/{id}` | 验证受众；返回可读内容或锁定预览 |
| `GET /api/posts/{id}/listing` | 作者恢复付费登记所需的合约及加密密钥参数 |
| `POST /api/posts/{id}/activate` | 作者请求核对合约登记并激活付费帖 |
| `POST /api/posts/{id}/purchase-info` | 检查受众、买家公钥和已有付款，准备付款信息 |
| `POST /api/posts/{id}/unlock` | 核对链上付款后交付正文和面向买家的付费密钥封装 |

JSON 内容请求最大 128 KiB；独立媒体路由最大 64 MiB，图片再限制 10 MiB。共用 NAS 请求并发及速率限制；内容操作总超时为 60 秒，接收者链查询最多 4 个并发。`401` 为会话失效，`402` 为尚未确认付款，`404` 合并不存在／无权限，`409` 为绑定／密钥／登记不匹配，`503` 为链或存储不可用。错误不回显私钥、令牌或带凭据的 RPC URL。

## 验证

```sh
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cd contracts && forge test
cd ../web && npm test && npm run build
# Local chain integration; build the NAS and contracts first. No real chain writes.
node --experimental-strip-types tests/publishing.anvil.mjs
```

Rust 测试覆盖 v2 数据库升级与重开、五类内容组合、关系范围签名快照、上传归属／元数据／完整性及付款前媒体隔离，以及签名／用途／过期、一次性提交、接收范围、定向隔离、密文持久化、付款前不释放正文／密钥、密钥重启与损坏保护。合约测试覆盖价格、命名空间、重复购买及收款失败回滚。独立 Anvil／NAS 测试实际部署合约，核对客户端 CBOR 与 EIP-712、登记、付费激活、购买、解锁及越权拒绝；未执行真实浏览器钱包扩展或测试网端到端验收。
