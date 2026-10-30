# 更新日志

本项目的所有重要变更都会记录在此文件中。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [未发布]

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
