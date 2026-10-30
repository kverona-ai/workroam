//! Output: human text for people, one stable JSON object (or JSON lines
//! for long jobs) for agents and scripts. Exit codes follow §15.

use crate::cli::ColorWhen;
use serde_json::{json, Value};
use std::io::{IsTerminal, Write};
use workroam_core::Error;
use workroam_ipc::SCHEMA_VERSION;

#[derive(Debug, Clone)]
pub struct Out {
    pub json: bool,
    pub quiet: bool,
    pub color: bool,
    /// stdout and stdin are terminals: a person is typing.
    pub interactive: bool,
    pub yes: bool,
    pub command: String,
}

impl Out {
    pub fn new(json: bool, quiet: bool, yes: bool, color: ColorWhen, command: &str) -> Out {
        let tty = std::io::stdout().is_terminal();
        let color = match color {
            ColorWhen::Always => true,
            ColorWhen::Never => false,
            ColorWhen::Auto => tty && std::env::var_os("NO_COLOR").is_none() && !json,
        };
        Out {
            json,
            quiet,
            color,
            interactive: tty && std::io::stdin().is_terminal() && !json,
            yes,
            command: command.to_string(),
        }
    }

    pub fn paint(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    pub fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }
    pub fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
    pub fn green(&self, s: &str) -> String {
        self.paint("32", s)
    }
    pub fn yellow(&self, s: &str) -> String {
        self.paint("33", s)
    }
    pub fn red(&self, s: &str) -> String {
        self.paint("31", s)
    }
    pub fn amber(&self, s: &str) -> String {
        self.paint("38;5;215", s)
    }

    /// Progress and notes for people (stderr); silent with --json / --quiet.
    pub fn info(&self, s: &str) {
        if !self.json && !self.quiet {
            let _ = writeln!(anstream::stderr(), "{s}");
        }
    }

    /// One JSON event line (long jobs, --json only).
    pub fn event(&self, v: Value) {
        if self.json {
            println!("{}", serde_json::to_string(&v).unwrap_or_default());
        }
    }

    /// Final result. `human` prints the text version to stdout.
    pub fn done(&self, result: &Value, human: impl FnOnce(&Out, &Value)) -> i32 {
        if self.json {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "ok": true,
                    "code": 0,
                    "command": self.command,
                    "result": result,
                }))
                .unwrap_or_default()
            );
        } else {
            human(self, result);
        }
        0
    }

    pub fn error(&self, e: &Error) -> i32 {
        let code = e.code.exit_code();
        if self.json {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "ok": false,
                    "code": code,
                    "command": self.command,
                    "error": e,
                }))
                .unwrap_or_default()
            );
        } else {
            let mut err = anstream::stderr();
            let _ = writeln!(err, "{} {}", self.red("error:"), e.message);
            if let Some(h) = &e.hint {
                let _ = writeln!(err, "  {} {h}", self.dim("hint:"));
            }
            if let Some(d) = &e.data {
                if let Some(list) = d.as_array() {
                    for x in list.iter().take(20) {
                        let _ = writeln!(
                            err,
                            "  - {}",
                            x.as_str()
                                .map(String::from)
                                .unwrap_or_else(|| x.to_string())
                        );
                    }
                }
            }
        }
        code
    }
}

pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

pub fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64()).unwrap_or(0)
}

pub fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}

pub fn time(v: &Value) -> String {
    v.as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "-".into())
}

pub fn summary_line(v: &Value) -> String {
    format!(
        "+{} ~{} -{} files, {}, {} commits",
        n(v, "added"),
        n(v, "modified"),
        n(v, "deleted"),
        workroam_core::duration::human_bytes(n(v, "bytes")),
        n(v, "commits")
    )
}
