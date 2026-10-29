# 架构决策记录（ADR）

每条记录描述一个影响架构的决定：背景、决定、后果。状态取值：**提议**、**已接受**、**已替代**。

| 编号 | 标题 | 状态 |
| --- | --- | --- |
| [0001](0001-client-heavy-s3-only.md) | 客户端重、服务端只用 S3 兼容存储 | 已接受 |
| [0002](0002-tray-tauri-with-language-neutral-ipc.md) | 托盘使用 Tauri v2，与后台服务之间使用语言无关的 IPC 协议 | 已接受 |
| [0003](0003-snapshot-engine-rustic-core.md) | 快照引擎使用 rustic_core | 已接受 |
| [0004](0004-per-machine-git.md) | 每台机器自有 `.git`，代码通过 git 自身传递 | 已接受 |

新增 ADR 时复制 [template.md](template.md)，编号递增，并更新上表。
