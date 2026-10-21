# 节点日志

NAS 与中继节点使用 `tracing` 记录英文运行日志。默认同时输出到标准错误和 `runtime/logs/`，路径相对于节点启动目录。现有节点升级后重启即可生效，不需要数据库迁移。

在根目录 `.env.profile` 配置以下选项；部署模板见 [.env.example](../.env.example) 和 [node.env.example](../deploy/node.env.example)。系统环境变量优先于配置文件。

| 配置项 | 默认值 | 说明 |
| --- | --- | --- |
| `RED_LEAVES_LOG_LEVEL` | `info` | 应用日志级别：`off`、`error`、`warn`、`info`、`debug`、`trace` |
| `RED_LEAVES_LOG_FORMAT` | `text` | `text` 或每行一条事件的 `json`；同时作用于控制台和文件 |
| `RED_LEAVES_LOG_CONSOLE` | `true` | 是否输出到标准错误；仅接受 `true` / `false` |
| `RED_LEAVES_LOG_FILE` | `true` | 是否输出文件；容器交由日志系统采集时可关闭 |
| `RED_LEAVES_LOG_DIR` | `runtime/logs` | 专用日志目录；缺失时自动创建 |
| `RED_LEAVES_LOG_MAX_FILES` | `7` | 每个节点角色最多保留的匹配日志文件数，范围为 1～365 |

文件按 UTC 日期轮转，例如 `red-leaves-nas.2026-10-21.log`、`red-leaves-relay.2026-10-21.log`。保留策略只清理同角色、同前后缀的日志文件，不会删除其他角色的日志或其他文件。数量限制不等于磁盘容量限制；单日日志仍可能增长，需要结合实际负载配置日志级别和磁盘监控。`off` 不创建日志文件。

Unix 下新建日志目录使用 `700` 权限。已有目录必须禁止组及其他用户访问，否则节点启动失败；程序不会自动修改已有目录权限，也不接受最终目录是符号链接。请使用独立日志目录，并保护其父目录及运行账号。文件受目录权限保护；日志仍包含节点监听地址、PeerId、帖子标识和运行状态，不应作为公开内容分享。

日志覆盖启动阶段、数据库初始化、HTTP 请求结果、链 RPC 耗时及失败、P2P 连接状态、投递进度和重试。HTTP 事件记录进程内请求编号、方法、路由模板、状态码及 `headers_ms`；该耗时截至响应头返回，不包含 SSE 等流式响应的整个持续时间。启动失败通过 `stage`（如 `nas_database`、`api_listener`）及安全的错误类别定位，避免直接输出可能含凭据的底层错误。

日志不记录请求正文、查询字符串、路径参数、认证头、签名、私钥、内容密钥或私密正文。应用级别调整不会开启依赖库的调试日志；依赖日志最高为 `warn`，SQLx 和底层 HTTP 客户端日志禁用，SQLite 语句日志也禁用。`RUST_LOG` 不覆盖这些规则。Panic 只记录源文件位置，不输出可能包含私密数据的 panic 文本。

文件写入使用独立线程和最多 2048 行的队列。队列满时丢弃文件日志，避免阻塞节点；正常退出时记录已丢弃的行数。收到 Ctrl+C 或 SIGTERM 时节点执行关闭并在退出时刷新文件队列。强制终止、磁盘故障或队列溢出仍可能丢失日志，因此这些运行日志不能作为可靠审计账本。

实现及轮转行为参考 [tracing-subscriber](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/index.html) 和 [tracing-appender](https://docs.rs/tracing-appender/latest/tracing_appender/rolling/struct.Builder.html)。
