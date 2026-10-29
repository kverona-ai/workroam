# ADR-0002：桌面应用使用 Tauri v2，与后台服务之间使用语言无关的 IPC 协议

- 状态：已接受
- 日期：2026-10-29

## 背景

桌面应用负责托盘图标、初始化与项目接入向导、授权 / 进度 / 完成 / 到达四种卡片和设置界面。卡片需要固定在屏幕角落、不抢键盘焦点、三系统一致，且必须稳定。

## 决定

桌面应用使用 Tauri v2 及其 tray、notification、autostart、positioner 插件。

桌面应用、CLI 与 MCP server 都只通过 IPC 与 `workroamd` 通信：换行分隔的 JSON-RPC 2.0，运行在 Unix domain socket 或 Windows 命名管道上，由 `workroam-ipc` crate 定义并导出 JSON Schema。桌面应用不包含任何同步逻辑。

## 后果

- 同步引擎与界面解耦，桌面应用可以独立演进或替换。
- 协议需要版本协商（`hello` 握手）并保持向后兼容。
- Linux 构建需要 WebKitGTK、AppIndicator 等系统库；GNOME 桌面需要 AppIndicator 扩展才能显示托盘图标。
