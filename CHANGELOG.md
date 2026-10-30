# 更新日志

本项目的所有重要变更都会记录在此文件中。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [未发布]

### 新增

- v0.1 文档：产品需求（`docs/prd.md`）、技术架构（`docs/architecture.md`）、桌面应用与命令行功能对照（`docs/cli-ui-parity.md`）、架构决策记录 ADR-0001 至 ADR-0009；全部功能在 v0.1 一次交付，验收门槛见 PRD §10
- 覆盖范围：首次初始化引导与检查、设备配对、项目接入与每台设备的项目目录、推送 / 拉取 / 租约、快照与恢复、项目归档与恢复、可选异地备份、代码仓库同步、六个本机 Agent 的适配与会话迁移、自动同步、命令行（含启动横幅）、Agent 接入（本机 Agent 用命令行 `--json` + Workroam Skill，云端个人 Agent 用 `workroam-server` 远程连接器）、可选的 `workroam-server`（中继模式可由 bot 部署在它的云端电脑上）、部署与安全
- 桌面应用高保真原型（`docs/prototype/`）与设计规范、Logo、README 产品示例图、命令行启动横幅示例（`docs/assets/cli-banner.svg`）
- README（默认英文，另有 `README.zh-CN.md` 中文版）、贡献指南、行为准则、安全策略、Issue 与 PR 模板
