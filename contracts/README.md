# 用户注册合约

使用 Foundry 和 Solidity 0.8.28，与 Rust、Web 独立构建。
`src/UserRegistry.sol` 已实现并通过本地测试；Base Sepolia 本地成功广播回执见 [部署记录](deployments/base-sepolia.json)，独立 RPC 复核状态注明其中。Web 已接入首次登记与钱包查询，NAS 提供持久 PeerId；流程见[首次绑定 NAS](../docs/nas-registration.md)。

## 数据与权限

钱包地址作为 mapping 查询键，只保存对应的 `nasPeerId`、`encryptionPublicKey`、`nickname`。
写入目标始终为 `msg.sender`，没有管理员代改、代理升级或服务密钥授权。昵称允许重名，多用户允许共用 NAS。
所有字段及历史交易公开；头像（含引用和哈希）、性别、私钥、令牌、中继入口均不写入本合约。

| 接口 | 行为 |
| --- | --- |
| `register(string,bytes,string)` | 钱包首次登记三个非空字段，重复登记拒绝 |
| `updateNasPeerId(string)` | 已登记钱包修改自己的 NAS PeerId |
| `updateEncryptionPublicKey(bytes)` | 已登记钱包修改自己的加密公钥 |
| `updateNickname(string)` | 已登记钱包修改自己的昵称 |
| `getUser(address)` | 返回三个字段；未登记时抛出 `NotRegistered` |
| `isRegistered(address)` | 查询是否已登记，不额外存储布尔字段 |

写入操作均发出相应事件，查询不发交易。NAS PeerId、昵称均为 1～128 字节，公钥为 1～256 字节。
长度按字节计算，不按字符；昵称由客户端使用 UTF-8 编码，合约不检查 UTF-8 有效性或规范化。
PeerId 与公钥按有界、不透明数据保存，不验证 PeerId 编码、公钥算法或密钥有效性，也不证明 NAS 在线。
使用前客户端须验证其格式；公钥算法、旧密文解密、多设备和密钥恢复协议仍待设计。
当前没有撤销、注销及原子修改整份资料的接口，不用空值隐式表达撤销。
修改公钥仅改变当前登记，不自动重加密历史内容或撤销已交付密钥。

## 验证

```sh
cd contracts
forge fmt --check
forge build
forge test
python3 -m unittest discover -s test -p 'deploy_config_test.py'
```

测试涵盖登记与查询、重复登记、未知用户、字段更新、用户隔离、重名和共用 NAS、
空值、最大长度、超长值、失败回滚、模糊输入和部署链检查。不依赖 forge-std 或外部合约库。

## 部署到 Base Sepolia

所有部署配置放在本目录的 `.env.profile`；模板为 `.env.example`。
已创建的 `.env.profile` 为本地忽略文件；不存在时执行：

```sh
cp .env.example .env.profile
chmod 600 .env.profile
```

在本地编辑以下配置，不要将真实私钥发到聊天或提交仓库：

```dotenv
BASE_SEPOLIA_RPC_URL=https://sepolia.base.org
DEPLOYER_PRIVATE_KEY=
```

`DEPLOYER_PRIVATE_KEY` 填写专用测试网部署钱包的 私钥（64 个十六进制数字，可带或不带 `0x` 前缀，脚本在内存中统一格式）。
部署钱包需持有 Base Sepolia 测试 ETH 支付 gas。部署者没有资料修改特权。
脚本从环境读取密钥，不把密钥或 RPC URL 作为命令行参数。
配置由 Bash 加载，使用普通 `KEY=value` 赋值即可。

先模拟，模拟成功后广播：

```sh
bash script/deploy.sh
bash script/deploy.sh --broadcast
```

模拟也需要 RPC 和部署钱包配置，但不会广播交易。脚本在链 ID 不等于 `84532` 时拒绝部署；
不会自动切换网络或跳过校验。不要使用 `-vvvv` 等额外调试参数输出敏感上下文。
默认不做区块浏览器源码验证，因此不需要浏览器 API key；源码验证可后续单独进行。

广播成功后，Foundry 在以下本地文件记录交易和回执：

```text
broadcast/DeployUserRegistry.s.sol/84532/run-latest.json
```

以成功回执为准，不能把模拟地址当成已部署地址。按 `deployments/README.md` 记录公开的地址、交易和区块；
NAS 后续需要配置所用链与该合约地址，不能仅靠合约名称确定权威注册表。

保存用户信息的合约地址（UserRegistry.sol）：
Base Sepolia 10-19 19:00: 0x8bace138A9A08009451001F62624960c581c5954

## 文本付费解锁合约

`src/PostAccess.sol` 是独立的 Base Sepolia 测试 ETH 付费原型。作者以 `(author, postId)` 登记固定价格及加密给自己 NAS 的内容密钥，买家通过 `buyPost(author, postId)` 付款，资金直接发送给作者，重复购买或错误金额被拒绝。任何链上存储都公开；不能把明文密钥或提前加密给未付款买家的密钥放入合约。

使用同一 `.env.profile` 配置，先执行 `bash script/deploy.sh --post-access` 模拟，审核后自行执行 `bash script/deploy.sh --post-access --broadcast`。默认不带该参数仍部署 UserRegistry；不会覆盖已有用户注册合约。将新地址填写到 NAS 根配置的 `RED_LEAVES_POST_ACCESS_ADDRESS`。

付款后由作者 NAS 核对链状态并向买家交付密钥，合约不承担解密或自动重新加密，不保证离线时交付。实现阶段未向测试网广播该合约；测试覆盖购买隔离、不可覆盖价格、重复购买、错误金额及作者收款失败回滚。完整流程和边界见[发帖说明](../docs/posting.md)。
