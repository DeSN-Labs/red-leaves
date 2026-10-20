# 部署记录

本地 Foundry 成功广播回执已记录到 [base-sepolia.json](base-sepolia.json)。本轮公共 RPC 在线复核返回 403，尚未独立复核链上字节码；记录注明证据来源。不要将本地测试或模拟地址作为测试网地址。

填写 `contracts/.env.profile` 并完成 `bash script/deploy.sh --broadcast` 后，
检查 `contracts/broadcast/DeployUserRegistry.s.sol/84532/run-latest.json` 中成功的交易回执，
再创建 `base-sepolia.json`，仅记录以下公开信息：

- chainId（84532）、contractName（UserRegistry）和 contractAddress；
- deploymentTransactionHash、blockNumber 和 deployer 钱包公开地址；
- compilerVersion（0.8.28）、源码版本或源码文件校验哈希；
- 已执行的构建／测试和部署日期。

不得记录私钥、RPC 密钥或整份环境配置。尚未执行部署时不预填地址或交易。
