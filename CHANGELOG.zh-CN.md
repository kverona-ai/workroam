# 更新日志

[English](CHANGELOG.md) · **简体中文**

本项目的所有重要变更都会记录在此文件中。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [未发布]

## [0.1.1] - 2026-10-30

v0.1.0 发布后对全部代码做了一轮全量 review（性能、设计、安全、功能完整性），本版本补齐发现的缺口。

### 新增

- 合盖 / 休眠 / 关机前推送（`sync.on_sleep`）真正生效：macOS 通过 IOKit 电源通知在系统休眠前最多等待 25 秒完成推送并释放租约；Windows 通过挂起 / 恢复通知、Linux 通过 logind 的 `PrepareForSleep` / `PrepareForShutdown` 触发（系统不等待，推送作业在唤醒后继续；网络中断导致失败时会像其他作业一样提示）；唤醒后立即检查有没有新进度（`sync.on_wake`）
- `workroamd` 收到 `SIGTERM`（注销 / 关机）或 Windows 关机 / 注销事件时有序退出：`sync.on_sleep = auto` 时先推送，再等待正在运行的作业最多 20 秒
- 拉取中断（进程崩溃、断电）后的恢复（架构 §9.3）：后台服务启动时重放操作日志、补完剩余步骤，已完成的步骤（包括已生成的冲突副本）不再重复；在此之前推送会被拒绝并提示先拉取；`workroam pull` 也会直接续做
- `.workroam/SYNCING` 标记记录写入它的进程，进程不在了就自动清除，不再让 Agent 一直暂停
- MCP 密钥：`.workroam/mcp.toml` 里 `${secret:name}` 引用的密钥现在可以保存到本机密钥库、删除，并按项目查看是否已设置——命令行 `workroam secret set / remove / list`，桌面应用在设置“Agent”→“MCP 密钥”；之前的提示指向一个不存在的命令

### 变更

- 快照保留文件与目录的修改时间：未改动的文件被上一个快照识别后不再重新读取与分块，大项目的推送明显变快；目录取其下最新文件的时间，未改动的目录树不再重复上传
- `workroam-server` 的 `/healthz` 只返回在线设备数量，不再列出设备名称
- 文档：新增中英双语的部署指南（`docs/deployment.md`）与配置说明（`docs/configuration.md`），安装文档补充英文版；README 指向正式 Release
- 发布说明默认英文，中文完整版折叠在“简体中文”里；更新日志拆分为英文 `CHANGELOG.md` 与中文 `CHANGELOG.zh-CN.md`
- 新增 “Release notes” 工作流，可为已发布的版本重新生成发布说明

### 修复

- 归档包（`.wrarchive`）解包：条目名与大小不再无条件信任（名称长度、相对路径、反斜杠与盘符检查，按实际读到的数据校验大小），损坏或恶意的文件不会导致巨大内存分配或写到目录之外
- 会话迁移：跳过含反斜杠、绝对路径或盘符的会话路径；Windows 与 Unix 路径前缀的大小写比较不再按字节切分（含非 ASCII 字符的路径不会崩溃）
- `workroam-server`：完整模式的签名请求在 5 分钟窗口内不可重放；OAuth 授权请求数量有上限（总计 20 个、每个客户端 3 个），防止未认证的调用方刷卡片；删除配对码的无效计数字段
- 时长解析（如 `sync.approval_timeout`）对溢出的数值报错而不是崩溃
- `doctor` 判断密钥库是否可用时真正探测系统密钥库，而不是一律报告“系统密钥库”
- 桌面应用：被包装成动作的构建函数返回的是 Promise 而不是元素，导致设置页里 Agent 区块（Agent 列表、MCP 开关、连接、webhook、默认代理设备）和异地备份区块的位置显示为 `[object Promise]`，接入项目时不显示检测到的仓库，归档恢复没有偏好选项，初始化向导的“发现已有工作区”和“密钥权限过宽”提示无法显示

## [0.1.0] - 2026-10-30

首个版本：PRD v0.1 的全部功能一次交付（验收门槛见 PRD §10）。

### 新增

- 桌面应用（Tauri v2，macOS / Windows / Linux）：托盘图标、首次初始化向导、主窗口与角落提示卡片，内含命令行 `workroam` 与后台服务 `workroamd`
- 命令行 `workroam` 与后台服务 `workroamd`：初始化与 `doctor` 检查、设备配对、项目接入、推送 / 拉取 / 租约、快照与恢复、历史、项目归档与恢复、可选异地备份、代码仓库同步、六个本机 Agent 的适配与会话迁移、自动同步；`--json` 输出供本机 Agent 通过 Workroam Skill 调用
- `workroam-server`：中继模式（单文件、musl 静态链接，可部署在 bot 的云端电脑上，供云端个人 Agent 经远程 MCP 连接器使用）与完整模式（配对码、每台设备独立密钥、通知转发、浏览器内解密的只读 Web 查看器）
- 自建存储部署：`deploy/install.sh` 一条命令部署 RustFS / Garage / SeaweedFS 与 Caddy，Docker Compose 片段，Kubernetes Helm Chart（`deploy/helm/workroam`）
- v0.1 文档：产品需求（`docs/prd.md`）、技术架构（`docs/architecture.md`）、桌面应用与命令行功能对照（`docs/cli-ui-parity.md`）、架构决策记录 ADR-0001 至 ADR-0009
- 桌面应用高保真原型（`docs/prototype/`）与设计规范、Logo、README 产品示例图、命令行启动横幅示例（`docs/assets/cli-banner.svg`）
- CI/CD：每个 PR 在三系统上检查格式、clippy、测试、`workroam-server` 静态构建与桌面应用构建；推送 `v*` tag 自动发布 Release，包含 macOS / Windows / Linux 的 x64 与 arm64 桌面安装包、命令行与 `workroam-server` 二进制、`ghcr.io` 容器镜像、源码包（含离线依赖版）、`SHA256SUMS` 与构建来源证明，以及安装脚本和 Homebrew / Scoop / winget 清单（`docs/install.md`、`docs/release.md`）
- README（默认英文，另有 `README.zh-CN.md` 中文版）、贡献指南、行为准则、安全策略、Issue 与 PR 模板
