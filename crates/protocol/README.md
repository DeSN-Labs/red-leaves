# red-leaves-protocol

NAS 节点通信协议的共享 Rust 库，依据[通信草案](../../docs/node-communication-protocol-v0.1.md)。
当前实现的是可测试的协议数据层，不启动 libp2p 网络，不读取环境配置，不访问链或数据库，不保存解密私钥。

## 已实现范围

| 模块 | 能力 |
| --- | --- |
| `types` | 网络范围、固定长度地址／nonce、SHA-256 multihash 内容标识、英文错误 |
| `wire` | Protobuf 消息及稳定字段编号；跨语言定义见 [red_leaves.proto](proto/red_leaves.proto) |
| `codec` | 无符号 varint 帧前缀、增量输入、控制／内容分流、长度和字段校验 |
| `canonical` | RFC 8949 core deterministic CBOR；签名对象字段、深度和元素数量检查 |
| `handshake` | Hello 校验、版本／能力与资源协商、绑定双方 PeerId 的会话 ID |
| `route` | libp2p SignedEnvelope 路由签名及验签、单跳中继地址、有效期、版本高水位 |
| `event` | EOA EIP-712 公开帖子／关注／点赞签名验证、事件 ID、语义去重键 |
| `receipt` | 按用户服务密钥签署保存声明，验证目标用户、NAS、事件和保存范围 |
| `file` | 2 GiB 内文件清单、最大 256 KiB 块、每页 128 个块哈希、页与块完整性检查 |

用户上下文消息只校验结构和能力名称。`grant` 暂为有界 opaque bytes，**不表示已验证钱包授权**。
设备授权、撤销传播、历史授权判定、链上公钥读取／轮换、HPKE／AEAD 格式及私钥保管尚未实现。
可选链下资料消息与真实中继支付未开放；未知消息／能力／签名算法明确拒绝，不降级为明文。

## 接入顺序与信任边界

1. 节点通过标准 libp2p 安全传输验证对端 PeerId；直连和中继采用相同的最终对端验证。
2. 按受信配置构造 `NetworkScope` 和 `Codec`，先调用 `decode_length` 校验长度，再分配／读取帧；完整缓冲区可直接用 `decode`。返回的 consumed 长度允许一次缓冲多个帧。
3. 交换 Hello，调用 `handshake::negotiate`，随后每帧都做 codec 校验和 `Negotiated::check_frame`。Hello 中的 nonce 由调用方使用安全随机源生成，每个连接重新生成，不能复用。
4. 节点查询当前链上 NAS 绑定，验证钱包签署的服务授权、会话持钥证明、撤销情况、本地接纳和用户策略。验证成功后才将 `context_id` 放入当前连接的用户上下文表；仅有一个格式正确的 context ID 不构成授权。
5. 使用 `event::verify` 验证作者签名，再核对 offer 作者／ID、上下文源用户、接收范围和黑名单。按网络和接收用户作用域持久化事件 ID 与语义去重键；库不维护全局去重缓存。
6. 请求对象和块时检查当前用户的访问权。`ChunkData.chunk_id` 自校验不能替代清单校验，必须再用 `FileManifest::verify_chunk` 匹配预期页／块索引与长度。全部收齐后流式验证整个文件的哈希和长度。
7. 文件及数据库真正持久化后才创建回执，使用用户服务密钥签名。接收回执时，公钥必须来自独立验证的有效授权，还需核对 `service_grant_ref` 和当时的绑定；中继签名或回执签名本身不能证明磁盘已经保存。

`Codec::decode` 不进行业务授权、事件验签、链查询、配额预留或持久化；`Negotiated::check_frame` 也不替代第 4 步。
库的大小上限是原型硬上限，节点接入时可用 `with_max_frame_bytes` 收紧，并按 `.env.profile` 实现运行策略。
握手超时、速率限制、并发信用、outbox／补同步游标和连接状态均由节点负责，库不宣称这些运行行为已经存在。

路由验证使用节点签名，不能由中继代签。`route::verify` 的 expected PeerId 来自钱包注册表或受信节点配置。
收到有效路由后调用 `checkpoint(previous)`，将结果持久化再采用；断线不能清除版本高水位。
DNS 路由只验证主机名形状，节点必须在每次解析／重连后对全部结果应用 `route::is_public_ip`；本地配对使用独立受信策略。

## 编码及签名约定

此实现作为首版原型的具体编码约定，尚无跨节点部署兼容性承诺：

- 网络 scope 使用协议族 `red-leaves`、network version `1`、链 ID 和注册合约地址。链及合约由调用方配置，不采用对端指定值。
- 控制协议 `/red-leaves/control/1.0.0`，内容协议 `/red-leaves/content/1.0.0`，Frame version `1`。
- Protobuf 采用最多 5 字节的最短 u32 varint 长度前缀，长度不含前缀。控制帧最多 64 KiB；事件／对象内容帧最多 288 KiB，块帧最多 272 KiB。Hello 和用户上下文控制消息不携带正文。
- 顶层 Frame 不允许重复 header、多份 oneof 或未知语义字段。Protobuf 子消息仍遵循标准解析规则；签名对象使用下面更严格的 CBOR 规则。
- Hello/HelloAck 无 session/context；OpenUserContext 有 session、无 context；业务消息携带两者。request ID 为非零 16 字节，session/context/nonce 为非零 32 字节。Hello 的版本与能力数组按数值严格升序。
- 签名 CBOR 使用文本 map 键，按其编码字节的字典序排列；最短无符号整数、定长 bytes／文本／数组／map、bool／null。拒绝负数、float、tag、重复键、不定长值、尾随数据、未知字段和省略后被补默认值的字段。最多嵌套 16 层、4096 个值。
- Rust 结构体字段名即 CBOR 文本键，enum 为标注的 snake_case 名称，地址／nonce／内容 ID 编为 byte string，可选字段明确编码为 `null`。所有签名对象保留原始规范字节，不从 JSON 重建。
- 内容 ID 为 `0x12 0x20 || SHA256(bytes)` 的 34 字节 multihash。事件 ID 是 `multihash(SHA256("red-leaves/event/1\0" || canonical_payload))`，其中 `\0` 为一个零字节；签名字节不参与事件 ID。
- EOA 事件域为 `EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)`，name=`RedLeaves`，version=`1`；类型为 `Event(bytes32 payloadHash)`，payloadHash=`keccak256(canonical_payload)`。不可使用 SIWE 或 personal-sign 替代。签名格式为 `r || s || v` 共 65 字节，仅接受 low-S 与 v=0/1/27/28。
- 公开事件通过 `PublicEvent` 明确表达，当前没有受限内容格式。Post 可携带正文及最多 32 个附件清单 ID；Follow 只携带目标钱包，Like 只携带目标事件 ID。作者时间仅供展示，不证明授权时间。
- 路由采用标准 libp2p SignedEnvelope，domain=`red-leaves/nas-route/1`，payload type=`/red-leaves/nas-route/1`；有效期最多 600 秒，允许 120 秒时钟偏差，最多 4 个 TCP／QUIC 单跳中继入口，完整封装最多 8 KiB。
- 回执 domain=`red-leaves/storage-receipt/1`，payload type=`/red-leaves/storage-receipt/1`，完整封装最多 8 KiB。正文保存、附件保存和用户已读分别处理，回执不代表已读。

语义去重键仅提供 Post ID、Follow(actor,target)、Like(actor,subject)；接入方仍需按用户／网络隔离并建立事务唯一约束，处理 nonce 重用和互动规则。此版本不提供取消关注／点赞、编辑或远程删除消息。

附件默认块大小为 256 KiB，可按协商上限收紧；每个清单仍最多 64 页／8192 块，因此更小的块会降低单文件可传输的长度上限。接入方创建清单时还须预留内容帧的封装空间。

## 验证

```sh
cargo test -p red-leaves-protocol --locked
cargo clippy -p red-leaves-protocol --all-targets --locked -- -D warnings
```

参考向量在 [tests/vectors](tests/vectors/)：公开事件摘要及签名已用 Web 的 viem 独立核对；Ping 帧由 `protoc --encode` 生成。日常 Rust 构建和测试不依赖 Node 或 protoc。
当前测试包括内存中的双 NAS 报文／事件／回执交换，不是实际 NAT 穿透、中继或多进程落盘测试。
