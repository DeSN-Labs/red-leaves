# 链上合约

使用 Foundry，与 Rust、Web 独立构建。当前只有目录和配置，尚无合约及部署。

```sh
cd contracts
forge fmt --check
forge build
forge test
```

空目录构建成功不表示注册协议已实现或经过测试。后续添加实际合约和测试后执行这些检查。
开发网络统一为 Base Sepolia（84532）；RPC 配置见 `.env.example`，使用前设置环境变量。
私钥不写入仓库；部署时采用本地签名工具或密钥库。
`deployments/` 记录未来各网络的合约地址、交易、区块及构建版本，不预填不存在的地址。
