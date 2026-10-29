# crates/

Rust 代码目录（Cargo workspace）。项目处于设计阶段，代码尚未开始。

规划中的 crate 及职责见 [技术架构 · 代码组织](../docs/architecture.md#4-代码组织)：

| crate | 职责 |
| --- | --- |
| `workroam-core` | 公共类型：项目、清单、配置模型、错误 |
| `workroam-store` | 存储访问（OpenDAL）、能力探测、条件写 |
| `workroam-lease` | 租约与最新快照指针 |
| `workroam-snapshot` | 快照引擎接口与 rustic_core 实现 |
| `workroam-git` | bundle、WIP、历史重建、三方合并 |
| `workroam-adapters` | 适配器加载、校验与本机文件生成 |
| `workroam-platform` | 跨平台检查、原子落盘、链接、阻止休眠、服务安装 |
| `workroam-ipc` | IPC 协议定义与传输 |
| `workroam-engine` | 推送 / 拉取流程编排 |
| `workroamd` | 后台服务 |
| `workroam-cli` | 命令行（二进制名为 `workroam`） |

托盘应用（Tauri v2）规划在 `apps/tray/`，适配器数据文件规划在 `adapters/`，部署配置规划在 `deploy/`。
