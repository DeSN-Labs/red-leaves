# 按范围推送：NAS、公共中继与实时收件箱

当前实现应用级推送 v1：已确认发布的帖子进入持久队列；NAS 常驻任务按签名受众投递；接收 NAS 过滤来源后保存免费正文或付费通知；Web 通过认证 SSE 自动刷新。支持同 NAS、多 NAS 直连，以及经过公共 Relay v2 中继的连接。公开广播表示向已连接网络公告，不能解释为全网每台设备都已收到。

## 配置

实际配置写入每个节点工作目录的 `.env.profile`，模板见根 `.env.example` 和 `deploy/node.env.example`。NAS 继续使用同一个 UserRegistry、Base Sepolia RPC 和可选 PostAccess。不同网络的注册合约地址不互通。

```dotenv
RED_LEAVES_P2P_ENABLED=true
RED_LEAVES_P2P_LISTEN=/ip4/0.0.0.0/tcp/4001
RED_LEAVES_P2P_PEERS=
RED_LEAVES_P2P_RELAYS=/dns4/relay.example.com/tcp/4001/p2p/<relay-peer-id>
RED_LEAVES_P2P_ADVERTISE=
RED_LEAVES_P2P_RELAY_SERVER=false
RED_LEAVES_RELAY_MAX_BYTES=134217728
```

- `P2P_PEERS` 填已知节点完整 multiaddr，以 `/p2p/<PeerId>` 结尾，用逗号分隔；用于直接连接与公开消息传播。每项配置最多 16 个节点。
- `P2P_RELAYS` 填公共中继的直接地址。NAT 后的 NAS 会申请、续约中继预留，通过 `relay-address/p2p-circuit/p2p/target-peer` 连接对方。首版要求双方使用至少一个共同的可达中继，或直接配置目标完整地址；PeerId 本身不能还原 IP。
- `P2P_ADVERTISE` 是对外可达的地址，例如 `/ip4/<public-ip>/tcp/4001`，不需要钱包地址。公共中继部署在 NAT／端口映射之后时必须填可达公网地址，不能把 `0.0.0.0` 当成公网地址。无配置时中继采用实际监听地址，适用于本机／可直连环境。
- `P2P_RELAY_SERVER=true` 可让 NAS 同时提供公共中继。独立中继使用 `cargo run -p red-leaves --no-default-features --features relay`，自动启用中继服务，需要配置相同 UserRegistry 作为网络命名空间，并设置独立的 `RED_LEAVES_NODE_KEY_PATH`。独立中继不提供用户 API，不保管应用私钥；PostgreSQL 持久存储、计费、票据结算尚未实现。
- 中继每条电路默认最多 128 MiB／300 秒，字节限额可设为 1 MiB～1 GiB；并发、预留及速率还受 libp2p 中继限制。NAS 运行日志显示实际 P2P 监听 multiaddr；需要从公网允许访问中继的 TCP 端口。
- 不需要远程推送时设置 `P2P_ENABLED=false`，同 NAS 投递和页面实时通知仍运行。未配置 UserRegistry 时不启动 P2P。

重启 NAS 后配置生效。`GET /api/nas/info` 返回真实 `p2p_running`、连接节点数 `p2p_peers` 和 `p2p_addresses`；`p2p_running=true` 只说明传输任务在运行，不证明目标可达。节点身份、用户密钥、数据库与内容目录必须保持一致。

## 发布范围与接收规则

| 发布范围 | 目标 |
| --- | --- |
| 全网广播 | GossipSub 公共通知；每台收到通知的 NAS 按其本地用户偏好决定是否获取正文 |
| 我关注的人 | 用户签名时确认的 following 钱包快照 |
| 关注我的人 | 用户签名时确认的 followers 钱包快照 |
| 好友 | 签名时互相关注的钱包交集 |
| 指定的人 | 签名确认的去重钱包名单 |

后四项都使用定向投递，不通过公共主题发布受众、标题或正文，最多 32 人。用户签名确认的是精确名单，签名后关系变化不会扩大发布权限。接收方先检查黑名单，再按 broadcasts／following／followers／direct 的 OR 组合接收；接收偏好不会扩大定向访问权或绕过付款。黑名单可以在“我的关注”管理，也可使用 `GET/POST /api/social/blocks`。

本 NAS 的接收者直接保存收件箱记录并通知客户端。跨 NAS 接收者的地址由链上最新 NAS PeerId 解析，连接由已知地址或公共中继提供。每次远程获取正文时，源 NAS 都从合约核对请求钱包绑定的 NAS 与经过 Noise 认证的连接 PeerId，不能只凭报文里的钱包地址取内容。

关注／取消关注同样通过独立签名动作同步给目标 NAS，使跨 NAS 的 followers 与互关范围可用。同步保存 `(owner,target,issued_at,nonce)` 检查点；准备动作时逐对单调增加签发时间，旧动作／重复投递不覆盖新状态。离线时重试最新状态。v3 以前未保存关注动作的签名，升级后旧本地关注不会凭空生成远端证明，需要用户再次签名关注一次。

## 投递、重启与页面更新

SQLite v4 新增持久收件箱、关注同步、黑名单和重试字段，自动升级 v3 并保留原帖子和任务。调度器最多并行处理 4 个出站任务，离线／超时按指数退避重试，间隔上限 300 秒；重启继续处理。目标更换 NAS 时重新读取链上绑定，不一直使用旧 PeerId。

`GET /api/posts/{id}/delivery` 仅作者可查，返回逐目标状态、尝试次数、下次时间和固定英文错误：

| 状态 | 含义 |
| --- | --- |
| `pending_network` / `retrying` | 等待或重试，不能显示已送达 |
| `delivered` | 目标 NAS 已校验签名并保存正文，或同 NAS 已保存投递记录 |
| `notified` | 目标已保存付费通知，尚未取得付费正文 |
| `filtered` | 接收方未选择该来源或已屏蔽，停止本次投递 |
| `announced` | 公共通知已交给连接网络，不代表所有接收者已保存 |

网络 ACK 经对端 PeerId 认证并在持久化后返回，是运行时的保存声明；尚未接入可独立转发验证的 `StorageReceipt` 签名封装，也不证明物理磁盘状态。每用户／帖子去重，重复投递不会增加副本或重复收费。付费帖必须先完成链上登记和激活，才进入投递队列。

公开通知每 120 秒重新公告；重新连接时拉取源节点最新最多 256 条公开通知补漏。首版不提供完整公开历史同步，也不枚举全部链上用户。明确拒收的定向旧帖不会因之后改偏好自动补投；改变接收偏好也不删除已存副本。

客户端使用 `GET /api/social/events` 的 Bearer SSE，令牌不放 URL、不使用 Cookie。初次连接和丢事件时同步收件箱，新消息只携带刷新信号；没有私密正文。每钱包最多 4 个订阅、NAS 最多 128 个；检查会话到期／退出，页面离开即取消，断线自动重连。关注动态按后端时间顺序显示同 NAS 帖子与收到的远程帖子。我的发布显示投递状态与明细。

## 签名、加密与付费

免费正文使用已签名 `social::Intent`。接收方核对确定性 CBOR、EOA 签名、内容 ID、作者、源 NAS、scope、价格和精确受众，不把历史帖子按提交挑战过期拒绝。源 NAS 只从已确认发布的持久对象提供内容。

跨 NAS 的正文响应再次用 AES-256-GCM 加密，并通过 HPKE 封装给接收钱包的最新链上公钥；AAD 为 `red-leaves/delivery/1/{post-id}/{lowercase-recipient}`。接收方只把该密文和封装落盘，NAS 应用私钥不离开盒子，浏览器离线时也可保存、校验正文。免费广播内容仍是公开内容，可以在授权连接中提供；不会将原有定向密文或私钥放入 GossipSub。

付费通知只含作者、帖子 ID、价格、传播类型和市场合约，不含真实标题、正文、媒体引用或密钥。接收方先核对作者当前 NAS 和作者命名空间内已确认的链上登记；这是付费公告，不是已验签明文。付款前不请求正文；解锁时本地和作者 NAS 都核对链上付款，作者 NAS 重新封装付费密钥，接收方此时才完整复核作者的签名 Intent 和受众，再保存加密正文。该方案仍依赖作者 NAS 在线且诚实交付，不实现无信任交付／自动退款。

媒体按需获取，不随通知复制。接收方从已验证的签名正文读取附件引用；源 NAS 检查受众、付款和调用 NAS 身份后交付接收者专用 HPKE 文件密钥封装，并产生绑定对端 PeerId 的随机下载能力。能力最长有效 120 秒、同节点最多 2 个、全 NAS 最多 64 个，最后一块读取后消费；过期／断线可重新申请。能力期间不逐块查链，因此不是即时撤销机制。

文件继续使用已落盘的 AES-GCM 密文，分块最多 256 KiB；接收方核对总长度、AEAD 和签名引用的 SHA-256 摘要后才返回给浏览器。中继看到端点及流量，看不到定向应用明文。正文可离线读缓存；未取回的媒体需要作者 NAS 在线，首版不持久缓存远程媒体或断点续传。

## 传输与兼容性

新增 `protocol::delivery` 描述应用级请求：`Offer`、`Fetch`、签名 `Follow`、`MediaStart`、`MediaChunk`、`Sync`；响应包括保存 ACK、忽略、加密正文、文件授权、块、公开通知页和固定错误码。使用 libp2p 0.56 TCP／DNS、Noise、Yamux、Relay v2、GossipSub；公开主题和请求流协议名都绑定链 ID 与 UserRegistry 地址，基础协议 `/red-leaves/delivery/1.0.0` 再追加 `/chain-id/registry-hex`。不同注册表不协商同一个应用协议。

单请求／响应最多 384 KiB，公共通知最多 8 KiB，出站待响应最多 32 个，应用命令／入站队列各最多 64 条，流并发最多 16，应用入站处理最多 8 个；请求超时、每 PeerId 的每分钟控制请求 120／块请求 1024、连接限额和有界收件箱共同限制资源消耗。每用户收件箱最多 10,000 条。

这是增量的应用传输版本，没有复用原型 Protobuf `Frame`／`HELLO`、DHT 签名路由发现、用户服务 grant、跨节点撤销或链网与社交共用 P2P；旧节点不支持该协议时任务保持重试，不降级为公开传输。公开中继暂不托管帖子，不实现流量收费、平台抽成或支付票据兑换。

## 验证

```sh
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build -p red-leaves --locked
cd web
npm test
npm run build
node --experimental-strip-types tests/delivery.anvil.mjs
```

集成测试仅在临时目录启动 Anvil、三台 NAS 和一个公共中继，使用开发账户；验证全部发布范围、跨 NAS 互关、SSE、偏好与黑名单、付款前隔离、付款后媒体分块、收发双方重启补投和离线正文缓存。不读取部署私钥、不写入真实测试网；公网多 NAT／长时间运行与真实浏览器扩展仍需环境验收。
