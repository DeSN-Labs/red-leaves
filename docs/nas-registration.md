# 首次绑定 NAS 与链上登记

已实现 NAS 节点身份持久化、认证后的身份查询、Web 首次登记与交易确认。
登录只证明钱包控制权，不自动上链；已实现直连／配置中继的 P2P 推送，多设备密钥同步尚未实现；图文／视频发布及加密见[发帖说明](posting.md)。

## 配置与启动

仓库根目录 `.env.profile` 为 NAS 配置，模板是根目录 `.env.example`。
节点启动时加载该文件；已有进程环境变量优先。默认从仓库根目录运行：

```sh
cargo run -p red-leaves
```

新增 `RED_LEAVES_NODE_KEY_PATH`，默认 `runtime/nas/node.key`。
NAS 首次启动生成 Ed25519 节点身份，以 libp2p protobuf 格式保存在文件中；Unix 权限为 600。
以后从同一文件恢复 PeerId。已存在但损坏的密钥导致启动失败，不自动覆盖或生成新身份。
备份和移动 NAS 必须保留密钥，建议生产配置使用绝对路径；不同工作目录下的相对路径可能产生不同身份。
写入异常可能留下不完整文件，此时须恢复有效备份，不得把删除文件当作普通重启步骤。

Web 配置放 `web/.env.profile`，模板为 `web/.env.example`：

```dotenv
VITE_API_BASE_URL=http://127.0.0.1:3000
VITE_USER_REGISTRY_ADDRESS=
```

第二项填写 Base Sepolia 上已部署的 UserRegistry 地址，不能填写钱包地址。
本地存在成功广播回执时，初始配置可取自 `contracts/broadcast/DeployUserRegistry.s.sol/84532/run-latest.json`。
本轮已用该本地回执准备合约配置；公共 RPC 在线复核返回 403，未据此宣称已在线核对字节码。
前端仍通过钱包校验链 ID、合约代码及接口查询；请核对自己部署的地址。
不要在任何 `VITE_` 变量中填写私钥或秘密。

```sh
cd web
npm ci
npm run dev
```

Vite 配置主动加载 `.env.profile` 的公开 API／合约地址，因此 IDE 默认 `development` 启动也可使用该配置；npm 脚本仍使用 `profile` mode。改配置后重启／重建。
链上读写通过浏览器钱包的 EIP-1193 provider，不要求 NAS 代签，也不使用前端的部署私钥或单独的 RPC 密钥。

## 操作流程

1. 钱包连接 Base Sepolia 并完成 SIWE 登录；用户仍可访问演示内容。
2. 登录后点击左下角钱包进入个人资料页查看登记状态，并行读取认证 NAS 的节点类型／PeerId 和本钱包的链上登记。
   RPC 错误、错误网络、缺失合约代码、配置缺失都显示错误，不按“未注册”处理。
3. 查询到未登记且认证接口明确返回 NAS 节点类型时，显示 `Register profile` 按钮；用户主动点击后填写昵称，并主动准备 NAS 持有的 X25519 公钥。
   页面展示钱包、合约、链及 NAS PeerId，确认信任 NAS 保管对应私钥后才能提交。
4. 再次验证 NAS 会话及 PeerId，检查钱包账户和链，模拟合约调用。
   只有明确点击登记按钮后才请求钱包确认 `register` 交易与 gas，不自动发送交易。
5. 保存交易哈希、目标钱包／合约及本次字段，等待两区块确认；成功回执后读取对应区块的登记，逐项验证一致才显示确认。
6. 用户拒签、模拟失败或交易回滚不会清除 NAS 登录。等待超时保留待确认交易，可点击继续检查。
   刷新后自动恢复同一钱包的有效会话即可读取待确认交易；会话过期时需重新登录；不会自动重发。钱包替换交易时跟踪新哈希，取消／回滚会报错。

已登记用户只显示链上资料。若 PeerId 与当前 NAS 不同，会提示差异，不自动更换 NAS 或重置公钥。
首次登记在本页面进行；个人资料页提供主动更新到 NAS 公钥的入口。更新 NAS／昵称、撤销绑定、完整轮换及最终性策略另行处理。
两区块确认是首期界面策略，不代表链最终性或保证不会重组；客户端读取依赖钱包所用 RPC。

## 节点身份接口

`GET /api/nas/info` 必须携带有效会话（Web Cookie／原生 Bearer），响应公开节点状态：

```json
{"node_type":"nas","peer_id":"<persistent NAS PeerId>","p2p_running":false,"p2p_peers":0,"p2p_addresses":[]}
```

不返回节点私钥或用户私钥。`p2p_running` 反映传输任务状态，连接数和监听地址便于检查可达性；见[推送配置](scoped-delivery.md)。
登记 PeerId 不代表节点已可通过 P2P 连接，也不自动获得其他用户数据访问权限。
该接口继承认证请求的 CORS、no-store、限流和超时约束。

## 内容加密密钥

当前 Web 首次登记使用 NAS 生成并保管的 X25519 密钥。认证接口 `GET /api/content/key` 返回原始 32 字节公钥、算法与保管方式，不返回私钥。私钥按用户加密保存；主密钥及密钥文件的备份规则见[发帖说明](posting.md)。合约仍只校验 1～256 字节长度，不替代算法、持钥或用户接纳校验。

Web 不再生成或保存浏览器内容加密私钥，也不导入旧浏览器密钥或旧密文。链上公钥必须与当前 NAS 持有的公钥匹配；不匹配时需用户主动确认更新交易，不能自动覆盖链上登记。发布时 NAS 会检查公钥与登记是否一致。

## 检查

```sh
cargo fmt --all --check
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cd web
npm test
npm run build
# 需要本地 Anvil 和已构建的 contracts/out/UserRegistry.sol/UserRegistry.json
node --experimental-strip-types --test tests/registry.anvil.mjs
```

自动测试覆盖持久节点身份与损坏保护、认证身份查询、浏览器密钥复用／隔离／不可导出、
登记字段边界、未注册与 RPC 错误区分、取消提交、错误网络、重复登记、待确认交易、真实本地合约确认及回滚。
尚未执行真实浏览器钱包扩展到 Base Sepolia 的端到端登记测试；本轮不代用户发送测试网注册交易。
