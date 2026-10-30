# Workroam 桌面应用

Tauri v2 外壳 + 无框架的 HTML/JS 界面，所有操作都通过 `workroamd` 的 IPC 完成（与命令行相同的方法），见 [技术架构 §15](../../docs/architecture.md) 与 [ADR-0009](../../docs/adr/0009-desktop-cli-feature-parity.md)。

```
apps/desktop/
├── actions.json   # 桌面端全部操作的一致性 ID（由 crates/workroam-ipc/tests/parity.rs 检查）
├── dev/serve.mjs  # 在普通浏览器里开发界面：把 /ipc 与 /events 转发到本机 workroamd
├── ui/            # 界面：index.html、style.css（来自高保真原型）、app.css、js/
└── src-tauri/     # Tauri 外壳：托盘、角落卡片窗口、守护进程桥接、自启动
```

## 开发

```sh
# 1. 启动后台服务（或让桌面应用自己启动）
cargo run --bin workroamd

# 2a. 在浏览器里开发界面（无需 Tauri），打开 http://127.0.0.1:1420/
node apps/desktop/dev/serve.mjs 1420
#     角落卡片：http://127.0.0.1:1420/#cards   审批详情：#details/<id>

# 2b. 或运行完整桌面应用
scripts/release/desktop-prepare.sh "" build   # 把 workroamd / workroam 放到 src-tauri/binaries/
cd apps/desktop && npx @tauri-apps/cli@^2 dev
```

Linux 需要 `libwebkit2gtk-4.1-dev`、`libayatana-appindicator3-dev`、`librsvg2-dev`（见 `.github/actions/linux-deps`）。

## 约定

- 每个用户操作都用 `act("<一致性 ID>", fn)` 包裹；新增操作时同时更新 `crates/workroam-ipc/parity.toml` 与 `actions.json`，再运行 `cargo xtask parity-doc`。
- 文案只放在 `ui/js/i18n.js`（中文、英文），样式遵循 `docs/prototype/DESIGN.md`。
- 托盘菜单在 `ui/js/tray.js` 里根据状态生成，由 `src-tauri/src/tray.rs` 绘制。
