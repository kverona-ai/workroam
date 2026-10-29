# ADR-0002：托盘使用 Tauri v2，与后台服务之间使用语言无关的 IPC 协议

- 状态：已接受
- 日期：2026-10-29

## 背景

托盘应用负责托盘图标与授权、进度、完成、到达四种卡片。卡片需要固定在屏幕角落、不抢键盘焦点、三系统一致。v0.1 的门槛是“连续两周不丢数据”，托盘必须稳定。

## 备选方案

| 方案 | 优点 | 缺点 |
| --- | --- | --- |
| Tauri v2 | 稳定；官方 tray、notification、autostart、positioner 插件；与 Rust 同栈 | 依赖系统 WebView；Linux 构建需要 WebKitGTK 等系统库 |
| GPUI | 纯 Rust、GPU 原生渲染、无 WebView；`WindowKind::PopUp` 适合角落卡片 | 尚未到 1.0，版本间常有破坏性变更；没有托盘 API，需要另接 `tray-icon`，Linux 上还需额外的 GTK 事件循环 |
| MyGo | 无 cgo，可交叉编译；原生 UI 或 WebView 均可 | 需要引入 Go；项目只有约一个月历史，API 变化快，维护者单一 |

## 决定

v0.1 使用 Tauri v2。托盘只通过 IPC 与 `workroamd` 通信，协议为换行分隔的 JSON-RPC 2.0，由 `workroam-ipc` crate 定义并导出 JSON Schema。托盘不包含任何同步逻辑。

## 后果

- 托盘实现可以在不改动同步引擎的前提下替换（例如 v0.2 之后评估 GPUI）。
- 协议需要版本协商（`hello` 握手）并保持向后兼容。
- CI 的 Linux 环境需要安装 Tauri 依赖的系统库。
