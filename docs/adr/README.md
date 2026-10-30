# 架构决策记录（ADR）

每条记录描述一个影响架构的决定：背景、决定、后果。状态取值：**提议**、**已接受**、**已替代**。

| 编号 | 标题 | 状态 |
| --- | --- | --- |
| [0001](0001-client-heavy-s3-only.md) | 客户端重、必需的服务端只有 S3 兼容存储 | 已接受 |
| [0002](0002-tray-tauri-with-language-neutral-ipc.md) | 桌面应用使用 Tauri v2，与后台服务之间使用语言无关的 IPC 协议 | 已接受 |
| [0003](0003-snapshot-engine-rustic-core.md) | 快照引擎使用 rustic_core | 已接受 |
| [0004](0004-per-machine-git.md) | 每台机器自有 `.git`，代码通过 git 自身传递 | 已接受 |
| [0005](0005-setup-wizard-desktop-first.md) | 首次初始化以桌面向导为主，命令行等价 | 已接受 |
| [0006](0006-pairing-code-encrypted-with-password.md) | 配对码用工作区加密密码保护 | 已接受 |
| [0007](0007-project-archive-and-optional-mirror.md) | 项目归档保存在同一个仓库中，异地备份只做可选的对象级镜像 | 已接受 |
| [0008](0008-agent-integration-cli-and-remote-connector.md) | 本机 Agent 以 CLI 为基础契约，云端个人 Agent 通过服务端中继的远程 MCP 连接器接入 | 已接受 |

新增 ADR 时复制 [template.md](template.md)，编号递增，并更新上表。
