# adapters/

Declarative adapter files, one per agent (architecture §12, ADPT-01). They are
compiled into the `workroam` binaries; a file with the same `id` in
`~/.workroam/adapters/` overrides the built-in one.

| File | Agent | Kind |
| --- | --- | --- |
| `claude-code.toml` | Claude Code | local |
| `codex.toml` | Codex | local |
| `grok-build.toml` | Grok Build | local |
| `antigravity.toml` | Antigravity | local |
| `zcode.toml` | ZCode | local |
| `pi.toml` | Pi | local |
| `grok-bot.toml` | Grok Bot | cloud |
| `muse.toml` | Muse | cloud |

`skills/workroam/SKILL.md` is the Workroam Skill template written to
`.agents/skills/workroam/SKILL.md` when a project is connected (AGT-02);
`skills/workroam/SKILL.cloud.md` is the variant for cloud personal agents.

Contributions welcome: copy an existing file, adjust paths and formats, and
add a case to `crates/workroam-adapters/tests/`.
