# 三台内网 NAS 推送测试

本测试使用 192.168.0.212、192.168.0.213、192.168.0.214 三台 Ubuntu 24.04 x86_64 服务器，每台运行一个 NAS。中继模式额外在 .212 运行独立的测试中继进程；直连模式只运行三台 NAS。测试客户端和临时 Anvil 在开发机运行，使用开发账户与临时 UserRegistry，不读取部署私钥、不访问真实链结算。

## 部署与端口

三台服务器的二进制均安装到 `/home/loop/red-leaves-lan-test/bin/red-leaves`。每轮使用独立的 `run-<timestamp>` 目录，保存 `.env.profile`、SQLite、节点身份、应用密钥、内容与日志；不会覆盖已有运行数据。测试结束后停止本轮节点、Anvil 及端口转发，保留远程文件以便排查；不是开机自启或持续运行的服务部署。

| 服务 | 监听地址 | 用途 |
| --- | --- | --- |
| NAS API | 各服务器 `127.0.0.1:3000` | 经 SSH 本地转发访问 |
| NAS P2P | 各服务器 `0.0.0.0:4001` | 实际 LAN 上的 Noise 加密连接 |
| 中继 API | .212 `127.0.0.1:3001` | 仅中继模式，查询测试中继身份 |
| 中继 P2P | .212 `0.0.0.0:4002` | 仅中继模式，Relay v2 电路 |
| 测试链转发 | 各服务器 `127.0.0.1:18545` | SSH 反向转发至开发机临时 Anvil |

.212 用于构建 Linux 二进制，安装了 build-essential、pkg-config、libssl-dev、cmake 及最小 Rust stable 工具链；另外两台只需运行二进制，不需安装 Rust。受下载速度影响使用 rsproxy 镜像，Rust 安装器核对了官方 SHA-256，Cargo 按现有 Cargo.lock 校验依赖。

构建源码仅包含根 Cargo.toml、Cargo.lock 和两个 Rust crate，不传输本地 `.env.profile`、数据库或真实密钥。构建命令为：

```sh
cargo build -p red-leaves --locked --config profile.dev.debug=0 --config profile.dev.incremental=false -j 2
```

这是关闭调试信息的开发构建，用于功能验收，不代表生产性能。

现有脚本已增加点赞汇集、当前权限和离线补投场景；下列 2026-10-23 实测结果仅针对当时的帖子／关注功能，不包含后来增加的点赞用例。当前 delivery v3 复测前须先部署新版本二进制。

## 2026-10-23 实测结果

使用本地工作区当前 Rust 源码（含未提交修改）构建，三台二进制 SHA-256 一致：

```text
7f91196ec94e79809af69e25fd5b9cfcd10640da36c60e2aea29ba3b53a07fa1
```

| 模式 | 远程轮次目录 | 结果 |
| --- | --- | --- |
| Relay v2 中继 | `run-1792767826383` | 所列功能场景全部通过 |
| NAS 内网直连 | `run-1792767961547` | 所列功能场景全部通过 |

直接读取各服务器只读数据库复核：两轮均由 .212 的 nas-0 发布 9 帖，.213 的 nas-1 收件箱 7 帖，.214 的 nas-2 收件箱 2 帖；源节点出站记录分别为 announced 2 条、delivered 6 条、filtered 1 条。广播的 announced 仅代表完成公告，广播实际接收另由收件箱断言验证。

结果目录位于各机 `/home/loop/red-leaves-lan-test/<轮次>/nas-0`、`nas-1` 或 `nas-2`；中继日志在 .212 同轮次的 `relay/logs/`，NAS 日志在对应 `nas-*/logs/`。源码和编译日志保留在 .212 的 `source/`、`build.log`。

验收后已确认三台均不再监听 3000、3001、4001、4002、18545 测试端口；远程配置、数据库、密钥与日志保留。临时测试链已经停止，这些目录不能直接作为持续运行环境使用；重跑脚本会新建测试链和独立轮次目录。

## 重跑

先确认各服务器已安装同一版本二进制、上述端口空闲、开发机可运行 Node.js、Anvil，且 `contracts/out/UserRegistry.sol/UserRegistry.json` 已通过 `forge build` 生成、Web 依赖已安装。

为三台机器分别建立 SSH 复用连接，交互输入密码；脚本不保存密码，也不修改 authorized_keys：

```sh
for host in 192.168.0.212 192.168.0.213 192.168.0.214; do
  ssh -M -S /tmp/red-leaves-lan-$host.sock -o ControlPersist=2h -fnN loop@$host
done
cp deploy/lan/.env.example deploy/lan/.env.profile
cd web
node --experimental-strip-types tests/delivery.anvil.mjs --lan=../deploy/lan/.env.profile
```

将 `deploy/lan/.env.profile` 的 `LAN_TRANSPORT` 从 `relay` 改为 `direct` 后，可用同一命令运行直连模式。配置模板见 [LAN 配置](../deploy/lan/.env.example)。默认在各服务器使用 `/home/loop/red-leaves-lan-test`；如果改变目录或用户名，需同步调整配置并部署二进制。

脚本覆盖三钱包注册、跨 NAS 互相关注、广播及 SSE、following／followers／friends／directed 范围、接收偏好、黑名单、非受众隔离、超过一个传输块的定向图片逐字节校验、离线重试、收发节点重启、PeerId 保持、保存 ACK，以及作者离线后读取已缓存正文。测试会主动停止和重启本轮 NAS，不能对生产目录运行。浏览器界面、真实钱包扩展、公网 NAT 穿透、真实链费用与长期稳定性不在该测试范围内。

全部完成后可关闭本次 SSH 复用连接：

```sh
for host in 192.168.0.212 192.168.0.213 192.168.0.214; do
  ssh -S /tmp/red-leaves-lan-$host.sock -O exit loop@$host
done
```
