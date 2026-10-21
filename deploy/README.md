# 部署配置

此目录保存节点部署配置模板，`node.env.example` 与根目录 `.env.example` 同步，实际配置写入根 `.env.profile`。NAS 认证、图文发帖及范围推送已实现，使用步骤见[发帖说明](../docs/posting.md)；直连及公共 Relay v2 的部署配置见[推送说明](../docs/scoped-delivery.md)；容器和 NAS 安装器尚未提供。
节点与 Web 独立构建，NAS 不需要安装 Node.js；未来可将 Web 静态产物随节点发布。
中继与家庭节点可以共用底层代码，但部署角色和管理权限必须分开。

节点默认同时输出控制台和按天轮转的文件日志，支持级别、JSON 格式、目录及保留数量配置，见[日志说明](../docs/logging.md)。容器采集标准错误时可设置 `RED_LEAVES_LOG_FILE=false`。
