# 参与贡献

感谢你愿意帮助 Workroam！本文说明如何提问题、提建议和提交代码。

> 项目目前处于设计阶段，最有价值的贡献是：对 [PRD](docs/prd.md) 和 [技术架构](docs/architecture.md) 提出问题，以及提供各 Agent 配置位置的实测信息。

## 行为准则

参与本项目即表示你同意遵守 [行为准则](CODE_OF_CONDUCT.md)。

## 提 Issue

- **Bug**：使用“Bug 报告”模板，附上系统、版本、复现步骤与 `workroam status` 输出（注意去掉敏感信息）。
- **功能建议**：使用“功能建议”模板，先说明要解决的问题，再说明方案。
- **Agent 适配**：使用“Agent 适配”模板，提供该 Agent 的规则、Skills、MCP、会话位置与版本号，最好附上官方文档链接。
- **安全问题**：不要公开提 Issue，请按 [SECURITY.md](SECURITY.md) 私下报告。

## 提交代码

1. Fork 仓库并从 `main` 创建分支，分支名建议使用 `feat/…`、`fix/…`、`docs/…`、`adapter/…`。
2. 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/zh-hans/)，例如：
   - `feat(cli): add workroam status`
   - `fix(git): keep WIP commit out of user index`
   - `docs(prd): clarify lease expiry`
   - `adapter(codex): support config.toml mcp section`
3. 每个提交都要带 DCO 签名（见下文）。
4. 打开 Pull Request，按模板填写说明；一个 PR 只做一件事。
5. 涉及架构变化时，请同时新增或更新 [ADR](docs/adr/README.md)。

### 代码规范（代码开始后适用）

- `cargo fmt --all` 与 `cargo clippy --all-targets -- -D warnings` 必须通过；
- 新功能需要测试；与存储相关的改动需要通过本地 RustFS 集成测试；
- 涉及路径、文件名、链接的改动，需要考虑 Windows、macOS、Linux 三个系统；
- 新增依赖需要通过 `cargo deny check`；
- 以上检查都在 CI 中自动运行，PR 需要 **CI OK** 通过才能合并；CI 与发布流程见 [持续集成与发布](docs/release.md)。

### 文档规范

- 文档放在 `docs/` 下，使用 Markdown，图表优先使用 Mermaid；
- 中文与英文、数字之间加空格；代码、路径、命令用反引号；
- 修改 PRD 中的需求编号时，同步更新架构文档中的引用。

## DCO（开发者原创声明）

本项目使用 [Developer Certificate of Origin](https://developercertificate.org/) 代替 CLA。提交时加上 `-s` 参数即可：

```bash
git commit -s -m "feat(cli): add workroam status"
```

提交信息末尾会自动附上：

```text
Signed-off-by: Your Name <you@example.com>
```

这表示你确认有权以本项目的许可证（Apache-2.0）提交这些内容。

## 许可证

除非另有说明，你提交的贡献将以 [Apache License 2.0](LICENSE) 授权。
