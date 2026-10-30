//! `.handoff/HANDOFF.md`: template and structured reader (AGT-03, `workroam handoff`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PATH: &str = ".handoff/HANDOFF.md";

/// Template sections: (key, heading).
pub const SECTIONS: &[(&str, &str)] = &[
    ("goal", "当前目标 / Goal"),
    ("done", "已完成 / Done"),
    ("in_progress", "进行中 / In progress"),
    ("next", "下一步 / Next steps"),
    ("questions", "待确认 / Open questions"),
    ("cautions", "注意事项 / Watch out"),
];

pub fn path(project_dir: &Path) -> PathBuf {
    project_dir.join(PATH)
}

pub fn template(project: &str) -> String {
    let mut s = format!(
        "# HANDOFF · {project}\n\n\
<!-- 推送前由当前 Agent 更新；保留以下标题。 / Updated by the current agent before every push; keep the headings. -->\n\n\
- 时间 / Time: \n- 设备 / Device: \n- Agent: \n"
    );
    for (_, h) in SECTIONS {
        s.push_str(&format!("\n## {h}\n\n- \n"));
    }
    s
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Handoff {
    pub exists: bool,
    pub time: Option<String>,
    pub device: Option<String>,
    pub agent: Option<String>,
    /// key → bullet items (empty placeholders dropped).
    pub sections: Vec<HandoffSection>,
    /// Whether the content still equals the empty template.
    pub empty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HandoffSection {
    pub key: String,
    pub heading: String,
    pub items: Vec<String>,
    pub text: String,
}

fn section_key(heading: &str) -> String {
    let h = heading.to_lowercase();
    for (k, t) in SECTIONS {
        let (zh, en) = t.split_once(" / ").unwrap_or((t, t));
        if h.contains(&zh.to_lowercase()) || h.contains(&en.to_lowercase()) {
            return k.to_string();
        }
    }
    h.split_whitespace().collect::<Vec<_>>().join("_")
}

pub fn parse(content: &str) -> Handoff {
    let mut h = Handoff {
        exists: true,
        ..Default::default()
    };
    let mut cur: Option<HandoffSection> = None;
    for line in content.lines() {
        let t = line.trim();
        if let Some(head) = t.strip_prefix("## ") {
            if let Some(s) = cur.take() {
                h.sections.push(s);
            }
            cur = Some(HandoffSection {
                key: section_key(head),
                heading: head.trim().to_string(),
                ..Default::default()
            });
            continue;
        }
        match cur.as_mut() {
            None => {
                let meta = t.trim_start_matches(['-', '*']).trim();
                if let Some((k, v)) = meta.split_once(':') {
                    let v = v.trim();
                    if v.is_empty() {
                        continue;
                    }
                    let k = k.to_lowercase();
                    if k.contains("时间") || k.contains("time") {
                        h.time = Some(v.into());
                    } else if k.contains("设备") || k.contains("device") {
                        h.device = Some(v.into());
                    } else if k.contains("agent") {
                        h.agent = Some(v.into());
                    }
                }
            }
            Some(s) => {
                if t.starts_with("<!--") {
                    continue;
                }
                if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
                    let item = item.trim();
                    if !item.is_empty() {
                        s.items.push(item.to_string());
                    }
                } else if t == "-" || t == "*" {
                } else if !t.is_empty() {
                    s.items.push(t.to_string());
                }
                if !(t == "-" || t.is_empty()) {
                    if !s.text.is_empty() {
                        s.text.push('\n');
                    }
                    s.text.push_str(line);
                }
            }
        }
    }
    if let Some(s) = cur.take() {
        h.sections.push(s);
    }
    h.empty = h.sections.iter().all(|s| s.items.is_empty());
    h
}

pub fn read(project_dir: &Path) -> Handoff {
    match std::fs::read_to_string(path(project_dir)) {
        Ok(s) => parse(&s),
        Err(_) => Handoff::default(),
    }
}

/// Create the template when missing. Returns true when written.
pub fn ensure(project_dir: &Path, project: &str) -> workroam_core::Result<bool> {
    let p = path(project_dir);
    if p.exists() {
        return Ok(false);
    }
    workroam_core::paths::write_atomic(&p, template(project).as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_empty_and_parses() {
        let h = parse(&template("p"));
        assert!(h.empty);
        assert_eq!(h.sections.len(), SECTIONS.len());
        let filled = template("p")
            .replace("- 时间 / Time: ", "- 时间 / Time: 2026-10-30 18:00")
            .replacen(
                "## 下一步 / Next steps\n\n- ",
                "## 下一步 / Next steps\n\n- 写测试",
                1,
            );
        let h = parse(&filled);
        assert!(!h.empty);
        assert_eq!(h.time.as_deref(), Some("2026-10-30 18:00"));
        let next = h.sections.iter().find(|s| s.key == "next").unwrap();
        assert_eq!(next.items, vec!["写测试"]);
    }
}
