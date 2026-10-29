# crates/

Rust 代码目录（Cargo workspace）。各 crate 的职责见 [技术架构 · 代码组织](../docs/architecture.md#4-代码组织)：

| crate | 职责 |
| --- | --- |
| `workroam-core` | 公共类型：工作区、项目、设备、清单、配置模型、错误 |
| `workroam-store` | 存储访问（OpenDAL）、能力探测、条件写 |
| `workroam-setup` | 初始化步骤与全部检查项 |
| `workroam-device` | 配对码、设备登记、凭证创建与轮换 |
| `workroam-lease` | 租约与最新快照指针 |
| `workroam-snapshot` | 快照引擎接口与 rustic_core 实现 |
| `workroam-archive` | 项目归档与恢复、归档包、异地备份镜像、完整性校验 |
| `workroam-git` | bundle、WIP、历史重建、三方合并 |
| `workroam-adapters` | 适配器加载、导入、本机文件生成、会话迁移 |
| `workroam-platform` | 跨平台检查、原子落盘、链接、阻止休眠、服务安装、密钥库 |
| `workroam-ipc` | IPC 协议定义与传输 |
| `workroam-engine` | 推送 / 拉取 / 恢复流程编排、触发策略 |
| `workroamd` | 后台服务 |
| `workroam-cli` | 命令行与 MCP server（二进制名为 `workroam`） |
| `workroam-server` | 可选服务端（v0.3） |

桌面应用在 `apps/desktop/`，Web 界面在 `apps/web/`，适配器数据文件在 `adapters/`，部署配置在 `deploy/`。
