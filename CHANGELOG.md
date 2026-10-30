# 更新日志

本项目的所有重要变更都会记录在此文件中。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [未发布]

### 新增

- 第一版完整产品需求文档（`docs/prd.md`）与技术架构文档（`docs/architecture.md`），覆盖最终形态 v0.3，包含首次初始化引导与检查流程
- 架构决策记录 ADR-0001 至 ADR-0008
- 项目目录：每个项目在每台设备上可自定义本地目录；目录为空时可更换，已同步后只能移除项目再重新添加（移除不删除文件）
- 项目归档与恢复：PRD 新增“项目归档与恢复”“回退与异地备份”用户旅程与 ARC、BAK 需求组，技术架构新增“归档与恢复”章节（从快照恢复、项目归档、恢复归档、归档包、可选异地备份）
- Agent 接入：区分本机 Agent 与云端个人 Agent。本机 Agent 以命令行 `--json` 加 Workroam Skill 接入，本机 MCP server 可选；Grok Bot、Muse 等云端个人 Agent 通过 `workroam-server` 的远程连接器（远程 MCP + OAuth，经设备中继执行）一键连接（PRD 7.8 与 AGT 需求组、架构 §16、ADR-0008）
- README（默认英文，另有 `README.zh-CN.md` 中文版）、贡献指南、行为准则、安全策略、Issue 与 PR 模板
