//! The desktop ↔ CLI feature list (`parity.toml`, architecture §17.4) and
//! the rendering of `docs/cli-ui-parity.md` from it.

use serde::Deserialize;

/// The manifest, compiled in so the desktop app and tests read the same list.
pub const MANIFEST: &str = include_str!("../parity.toml");

#[derive(Debug, Deserialize)]
pub struct Parity {
    pub header: String,
    #[serde(rename = "section")]
    pub sections: Vec<Section>,
}

#[derive(Debug, Deserialize)]
pub struct Section {
    pub title: String,
    #[serde(rename = "block")]
    pub blocks: Vec<Block>,
}

#[derive(Debug, Deserialize)]
pub struct Block {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default, rename = "row")]
    pub rows: Vec<Row>,
}

#[derive(Debug, Deserialize)]
pub struct Row {
    #[serde(default)]
    pub id: Option<String>,
    pub cells: Vec<String>,
    /// `desktop` or `cli`: an item of the exemption list.
    #[serde(default)]
    pub exempt: Option<String>,
    #[serde(default)]
    pub ipc: Vec<String>,
    #[serde(default)]
    pub cli: Vec<String>,
    #[serde(default)]
    pub desktop: Vec<String>,
    /// Why `desktop` is empty: an item of the CLI-only exemption list.
    #[serde(default)]
    pub desktop_exempt: Option<String>,
    #[serde(default)]
    pub since: Option<String>,
}

impl Parity {
    pub fn load() -> Result<Parity, String> {
        Self::parse(MANIFEST)
    }

    pub fn parse(s: &str) -> Result<Parity, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }

    /// Every feature row (not the exemption list).
    pub fn features(&self) -> impl Iterator<Item = &Row> {
        self.sections
            .iter()
            .flat_map(|s| s.blocks.iter())
            .flat_map(|b| b.rows.iter())
            .filter(|r| r.exempt.is_none())
    }

    pub fn exempt(&self, side: &str) -> impl Iterator<Item = &Row> + '_ {
        let side = side.to_string();
        self.sections
            .iter()
            .flat_map(|s| s.blocks.iter())
            .flat_map(|b| b.rows.iter())
            .filter(move |r| r.exempt.as_deref() == Some(side.as_str()))
    }

    /// `docs/cli-ui-parity.md`.
    pub fn render(&self) -> String {
        let mut out = self.header.clone();
        let sections: Vec<String> = self
            .sections
            .iter()
            .map(|s| {
                let blocks: Vec<String> = s.blocks.iter().map(render_block).collect();
                format!("## {}\n\n{}\n", s.title, blocks.join("\n\n"))
            })
            .collect();
        out.push_str(&sections.join("\n"));
        out
    }
}

fn render_block(b: &Block) -> String {
    if let Some(t) = &b.text {
        return t.clone();
    }
    let mut lines = vec![
        format!("| {} |", b.columns.join(" | ")),
        format!("| {} |", vec!["---"; b.columns.len()].join(" | ")),
    ];
    for r in &b.rows {
        lines.push(format!("| {} |", r.cells.join(" | ")));
    }
    lines.join("\n")
}

/// Split a manifest `cli` example into arguments (no shell quoting needed).
pub fn cli_args(example: &str) -> Vec<String> {
    example.split_whitespace().map(String::from).collect()
}
