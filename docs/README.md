# 文档

| 文档 | 内容 |
| --- | --- |
| [产品需求（PRD）](prd.md) | v0.1 完整功能：目标用户、用户旅程（含首次初始化引导）、功能需求、非功能需求、发布范围与验收门槛 |
| [技术架构](architecture.md) | 组件、代码组织、存储布局、初始化与检查、设备与凭证、同步协议、git 同步、Agent 适配、IPC、Agent 接入（CLI、本机 MCP、云端个人 Agent 远程连接器）、可选服务端、安全、部署、测试 |
| [架构决策记录](adr/README.md) | 重要技术决策的背景、决定与后果 |
| [桌面应用与命令行功能对照](cli-ui-parity.md) | 每项功能在桌面应用（向导、主窗口、卡片、托盘）与 `workroam` 命令行中的入口，以及豁免清单 |
| [安装](install.md) | 桌面应用、命令行、`workroam-server` 的下载与安装，校验下载，从源码构建 |
| [持续集成与发布](release.md) | CI 检查项、发布流程、Release 产物、需要配置的 secrets 与包管理器渠道 |
| [高保真原型](prototype/README.md) | 桌面应用的可点击原型（Liquid Glass 风格）与 Logo 设计说明 |

## 阅读顺序

1. PRD 的 [§1 产品概述](prd.md#1-产品概述) 与 [§7 用户旅程](prd.md#7-用户旅程)：了解 Workroam 解决什么问题、用户怎么用；
2. 技术架构的 [§2 系统上下文](architecture.md#2-系统上下文)、[§7 初始化引导与检查](architecture.md#7-初始化引导与检查) 与 [§9 同步协议](architecture.md#9-同步协议)；
3. 具体选型的理由见 [ADR](adr/README.md)。

## 资源

- `assets/logo.svg`：项目 Logo（设计说明见 [prototype/README.md](prototype/README.md#logo)）
- `prototype/assets/logo-mono.svg`：菜单栏 / 托盘用单色图标
- `assets/screenshots/`：README 中的产品示例图，截取自高保真原型
- `assets/cli-banner.svg`：命令行启动横幅示例（设计见 [技术架构 §17.3](architecture.md#173-启动横幅)）
