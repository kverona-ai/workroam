//! Import existing agent configuration into the shared sources
//! (`workroam init`, architecture §12.3): rule files into `AGENTS.md`, native
//! MCP configs into `.workroam/mcp.toml`, agent skill folders into `.agents/skills/`.

use crate::generate::parse_mcp;
use crate::model::Adapter;
use serde::{Deserialize, Serialize};
use std::path::Path;
use workroam_core::config::McpConfig;
use workroam_core::Result;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    Rules,
    Mcp,
    Skill,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportItem {
    pub kind: ImportKind,
    pub agent: String,
    /// Source path, project relative.
    pub source: String,
    /// Name (MCP server or skill) when relevant.
    pub name: Option<String>,
    /// Another definition with the same name differs: the user must choose.
    pub conflict: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImportPlan {
    pub items: Vec<ImportItem>,
}

impl ImportPlan {
    pub fn conflicts(&self) -> impl Iterator<Item = &ImportItem> {
        self.items.iter().filter(|i| i.conflict)
    }
}

fn is_template_only(content: &str, adapter: &Adapter, rel: &str) -> bool {
    let t = content.trim();
    t.is_empty()
        || adapter
            .rules
            .generate
            .iter()
            .any(|g| g.path == rel && g.template.trim() == t)
        || t == "@AGENTS.md"
}

/// Look at what exists and describe what an import would do.
pub fn plan(project_dir: &Path, adapters: &[Adapter]) -> Result<ImportPlan> {
    let mut plan = ImportPlan::default();
    let agents_md = std::fs::read_to_string(project_dir.join("AGENTS.md")).unwrap_or_default();
    let mcp = McpConfig::load(project_dir)?;
    for a in adapters.iter().filter(|a| a.is_local()) {
        for rel in &a.rules.import {
            if let Ok(c) = std::fs::read_to_string(project_dir.join(rel)) {
                if is_template_only(&c, a, rel) || agents_md.contains(c.trim()) {
                    continue;
                }
                plan.items.push(ImportItem {
                    kind: ImportKind::Rules,
                    agent: a.id.clone(),
                    source: rel.clone(),
                    name: None,
                    conflict: false,
                    detail: format!("{} lines → AGENTS.md", c.lines().count()),
                });
            }
        }
        if let Some(m) = a.mcp.as_ref().filter(|m| m.import) {
            if let Ok(c) = std::fs::read_to_string(project_dir.join(&m.path)) {
                for (name, def) in parse_mcp(&m.format, &c).unwrap_or_default() {
                    if name == "workroam" {
                        continue;
                    }
                    let existing = mcp.servers.get(&name);
                    let same = existing.map(|e| {
                        e.command == def.command && e.args == def.args && e.url == def.url
                    });
                    if same == Some(true)
                        || plan
                            .items
                            .iter()
                            .any(|i| i.kind == ImportKind::Mcp && i.name.as_deref() == Some(&name))
                    {
                        continue;
                    }
                    plan.items.push(ImportItem {
                        kind: ImportKind::Mcp,
                        agent: a.id.clone(),
                        source: m.path.clone(),
                        name: Some(name.clone()),
                        conflict: existing.is_some(),
                        detail: def.url.clone().unwrap_or_else(|| {
                            format!(
                                "{} {}",
                                def.command.clone().unwrap_or_default(),
                                def.args.join(" ")
                            )
                            .trim()
                            .to_string()
                        }),
                    });
                }
            }
        }
        if let Some(sk) = &a.skills {
            if let Some(link) = &sk.link {
                let dir = project_dir.join(link);
                let is_link = std::fs::symlink_metadata(&dir)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
                    || workroam_platform::fsutil::is_junction(&dir);
                if is_link {
                    continue;
                }
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten().filter(|e| e.path().is_dir()) {
                        let name = e.file_name().to_string_lossy().to_string();
                        let target = project_dir.join(&sk.source).join(&name);
                        plan.items.push(ImportItem {
                            kind: ImportKind::Skill,
                            agent: a.id.clone(),
                            source: format!("{link}/{name}"),
                            name: Some(name),
                            conflict: target.exists(),
                            detail: format!("→ {}", sk.source),
                        });
                    }
                }
            }
        }
    }
    Ok(plan)
}

/// How to resolve a conflicting item.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// Keep what is already in the shared sources.
    #[default]
    KeepExisting,
    /// Take the agent's version.
    TakeImported,
}

/// Apply a plan. Original files are copied to `.workroam/local/imported/` first.
pub fn apply(
    project_dir: &Path,
    adapters: &[Adapter],
    plan: &ImportPlan,
    choice: &dyn Fn(&ImportItem) -> Choice,
) -> Result<Vec<String>> {
    let mut done = Vec::new();
    let backup = project_dir.join(".workroam/local/imported");
    let mut agents_md = std::fs::read_to_string(project_dir.join("AGENTS.md")).unwrap_or_default();
    let mut agents_changed = false;
    let mut mcp = McpConfig::load(project_dir)?;
    let mut mcp_changed = false;
    for item in &plan.items {
        if item.conflict && choice(item) == Choice::KeepExisting {
            continue;
        }
        let Some(a) = adapters.iter().find(|a| a.id == item.agent) else {
            continue;
        };
        let src = project_dir.join(&item.source);
        match item.kind {
            ImportKind::Rules => {
                let c = std::fs::read_to_string(&src)?;
                let b = backup.join(&item.source);
                if let Some(p) = b.parent() {
                    std::fs::create_dir_all(p)?;
                }
                std::fs::write(&b, &c)?;
                let body = crate::envblock::strip(&agents_md);
                let mut out = body.trim_end().to_string();
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(&format!(
                    "<!-- 导入自 / imported from {} -->\n{}\n",
                    item.source,
                    c.trim()
                ));
                agents_md = out;
                agents_changed = true;
                // The file is regenerated as a reference to AGENTS.md.
                if let Some(g) = a.rules.generate.iter().find(|g| g.path == item.source) {
                    std::fs::write(&src, &g.template)?;
                }
                done.push(format!("{} → AGENTS.md", item.source));
            }
            ImportKind::Mcp => {
                let m = a.mcp.as_ref().expect("mcp item without mcp section");
                let c = std::fs::read_to_string(&src)?;
                if let Some(def) =
                    parse_mcp(&m.format, &c)?.remove(item.name.as_deref().unwrap_or_default())
                {
                    mcp.servers
                        .insert(item.name.clone().unwrap_or_default(), def);
                    mcp_changed = true;
                    done.push(format!(
                        "{} ({}) → .workroam/mcp.toml",
                        item.name.clone().unwrap_or_default(),
                        item.source
                    ));
                }
            }
            ImportKind::Skill => {
                let sk = a
                    .skills
                    .as_ref()
                    .expect("skill item without skills section");
                let name = item.name.clone().unwrap_or_default();
                let target = project_dir.join(&sk.source).join(&name);
                if target.exists() {
                    workroam_platform::fsutil::remove_any(&target)?;
                }
                workroam_platform::fsutil::copy_dir(&src, &target)?;
                let b = backup.join(&item.source);
                if let Some(p) = b.parent() {
                    std::fs::create_dir_all(p)?;
                }
                workroam_platform::fsutil::copy_dir(&src, &b)?;
                done.push(format!("{} → {}/{}", item.source, sk.source, name));
            }
        }
    }
    // Skill folders that were copied become links on the next generation.
    for item in plan.items.iter().filter(|i| i.kind == ImportKind::Skill) {
        if !(item.conflict && choice(item) == Choice::KeepExisting) {
            if let Some(link) = adapters
                .iter()
                .find(|a| a.id == item.agent)
                .and_then(|a| a.skills.as_ref())
                .and_then(|s| s.link.clone())
            {
                let p = project_dir.join(&link);
                if p.is_dir()
                    && !std::fs::symlink_metadata(&p)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false)
                {
                    let _ = std::fs::remove_dir_all(&p);
                }
            }
        }
    }
    if agents_changed {
        workroam_core::paths::write_atomic(&project_dir.join("AGENTS.md"), agents_md.as_bytes())?;
    }
    if mcp_changed {
        mcp.save(project_dir)?;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_claude_files() {
        let td = tempfile::tempdir().unwrap();
        let d = td.path();
        std::fs::write(d.join("CLAUDE.md"), "Use tabs.\n").unwrap();
        std::fs::write(
            d.join(".mcp.json"),
            r#"{"mcpServers":{"gh":{"command":"gh-mcp","args":["serve"]}}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(d.join(".claude/skills/review")).unwrap();
        std::fs::write(d.join(".claude/skills/review/SKILL.md"), "x").unwrap();
        let adapters = vec![crate::by_id("claude").unwrap()];
        let p = plan(d, &adapters).unwrap();
        assert_eq!(p.items.len(), 3);
        assert_eq!(p.conflicts().count(), 0);
        apply(d, &adapters, &p, &|_| Choice::KeepExisting).unwrap();
        assert!(std::fs::read_to_string(d.join("AGENTS.md"))
            .unwrap()
            .contains("Use tabs."));
        assert_eq!(
            std::fs::read_to_string(d.join("CLAUDE.md")).unwrap(),
            "@AGENTS.md\n"
        );
        assert!(McpConfig::load(d).unwrap().servers.contains_key("gh"));
        assert!(d.join(".agents/skills/review/SKILL.md").exists());
        // Nothing left to import.
        let again = plan(d, &adapters).unwrap();
        assert!(again
            .items
            .iter()
            .all(|i| i.kind != ImportKind::Rules && i.kind != ImportKind::Mcp));
    }
}
