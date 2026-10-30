//! Bringing a folder under Workroam (`workroam init`, PRD §7.2) and the
//! per-device project bindings (`project add / move / remove`, §6.7).

use crate::project::{default_config, Project};
use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use workroam_adapters::import::{self, Choice, ImportPlan};
use workroam_adapters::{envblock, handoff};
use workroam_core::config::{
    AgentToggle, BindingState, MachineConfig, McpConfig, ProjectBinding, ProjectConfig, RepoConfig,
};
use workroam_core::{Code, Error, Result};
use workroam_git::scan::FoundRepo;
use workroam_platform::dircheck::DirIssue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitPlan {
    pub project: String,
    pub dir: String,
    pub already_initialized: bool,
    pub repos: Vec<FoundRepo>,
    pub imports: ImportPlan,
    pub agents: Vec<workroam_adapters::Detected>,
    pub dir_issues: Vec<DirIssue>,
    pub problems: Vec<workroam_platform::xp::XpProblem>,
    /// Another project of this workspace already uses the name.
    pub name_taken: bool,
}

/// `others`: directories of the other projects bound on this device.
pub fn other_dirs(m: &MachineConfig, except: &str) -> Vec<PathBuf> {
    m.projects
        .iter()
        .filter(|(n, _)| n.as_str() != except)
        .map(|(_, b)| workroam_core::paths::expand_tilde(&b.path))
        .collect()
}

pub async fn plan_init(ws: &Workspace, name: &str, dir: &Path) -> Result<InitPlan> {
    workroam_core::config::validate_project_name(name)?;
    let dir = workroam_core::paths::normalize(dir);
    let everywhere = reachable_everywhere(ws).await;
    let here = ws.machine.reachable.clone();
    let rels = workroam_git::scan::find_repos(&dir);
    let mut repos: Vec<FoundRepo> = rels
        .iter()
        .map(|r| {
            workroam_git::scan::recommend(&dir, r, &|h| here.iter().any(|x| x == h), &|h| {
                everywhere.iter().any(|x| x == h)
            })
        })
        .collect();
    // The project root itself may be a repository.
    if dir.join(".git").exists() {
        let mut r =
            workroam_git::scan::recommend(&dir, ".", &|h| here.iter().any(|x| x == h), &|h| {
                everywhere.iter().any(|x| x == h)
            });
        r.name = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "root".into());
        // The project root is synced file by file; its git history stays per device.
        r.recommended.mode = workroam_core::model::RepoMode::Files;
        r.recommended.history = String::new();
        r.recommended.via = None;
        r.reason =
            "项目根目录本身是仓库，按文件同步 / the project root is a repository: files mode"
                .into();
        repos.insert(0, r);
    }
    let adapters = workroam_adapters::registry::local();
    let imports = import::plan(&dir, &adapters)?;
    let agents = workroam_adapters::registry::all()
        .iter()
        .map(workroam_adapters::detect)
        .collect();
    let dir_issues =
        workroam_platform::dircheck::check_project_dir(&dir, &other_dirs(&ws.machine, name));
    let files: Vec<String> = walkdir::WalkDir::new(&dir)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git" && e.file_name() != "node_modules")
        .flatten()
        .filter_map(|e| workroam_core::paths::to_snapshot_path(&dir, e.path()))
        .filter(|p| !p.is_empty())
        .take(200_000)
        .collect();
    let windows_root = if ws.machine.os == workroam_core::model::OsKind::Windows {
        dir.display().to_string().len()
    } else {
        0
    };
    let problems =
        workroam_platform::xp::check_paths(files.iter().map(|s| s.as_str()), windows_root);
    let name_taken = ws.head(name).await?.is_some()
        && crate::local::Binding::load(&dir)
            .map(|b| b.project != name)
            .unwrap_or(true);
    Ok(InitPlan {
        project: name.to_string(),
        dir: dir.display().to_string(),
        already_initialized: ProjectConfig::load(&dir)?.is_some(),
        repos,
        imports,
        agents,
        dir_issues,
        problems,
        name_taken,
    })
}

/// Hosts every read-write device can reach (for remote-mode recommendations).
pub async fn reachable_everywhere(ws: &Workspace) -> Vec<String> {
    let devices = workroam_device::registry::list(&ws.control)
        .await
        .unwrap_or_default();
    let mut common: Option<Vec<String>> = Some(ws.machine.reachable.clone());
    for d in devices.iter().filter(|d| d.id != ws.machine.device_id) {
        common = common.map(|c| c.into_iter().filter(|h| d.reachable.contains(h)).collect());
    }
    common.unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InitChoices {
    /// Repository settings by name; missing ones use the recommendation.
    #[serde(default)]
    pub repos: BTreeMap<String, RepoConfig>,
    /// Repositories to leave out.
    #[serde(default)]
    pub skip_repos: Vec<String>,
    /// Conflicting imports: take the agent's version instead of keeping the existing one.
    #[serde(default)]
    pub take_imported: bool,
    /// Enabled agents (default: the installed ones).
    #[serde(default)]
    pub agents: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InitOutcome {
    pub project: String,
    pub dir: String,
    pub written: Vec<String>,
    pub imported: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn apply_init(ws: &Workspace, plan: &InitPlan, choices: &InitChoices) -> Result<InitOutcome> {
    if plan.dir_issues.iter().any(|i| i.blocking) {
        let i = plan.dir_issues.iter().find(|i| i.blocking).unwrap();
        return Err(Error::new(Code::InvalidArgs, i.message.clone()));
    }
    if plan.name_taken {
        return Err(Error::new(
            Code::InvalidArgs,
            format!("the workspace already has a project named {}", plan.project),
        )
        .with_hint(format!(
            "add it on this device with: workroam project add {}",
            plan.project
        )));
    }
    let dir = PathBuf::from(&plan.dir);
    std::fs::create_dir_all(&dir)?;
    let mut out = InitOutcome {
        project: plan.project.clone(),
        dir: plan.dir.clone(),
        ..Default::default()
    };
    let adapters = workroam_adapters::registry::local();
    let choice = |_: &import::ImportItem| {
        if choices.take_imported {
            Choice::TakeImported
        } else {
            Choice::KeepExisting
        }
    };
    out.imported = import::apply(&dir, &adapters, &plan.imports, &choice)?;

    let mut cfg = ProjectConfig::load(&dir)?.unwrap_or_else(|| default_config(&plan.project));
    cfg.name = plan.project.clone();
    for r in &plan.repos {
        if choices.skip_repos.contains(&r.name) || cfg.repos.contains_key(&r.name) {
            continue;
        }
        let rc = choices
            .repos
            .get(&r.name)
            .cloned()
            .unwrap_or_else(|| r.recommended.clone());
        cfg.repos.insert(r.name.clone(), rc);
    }
    if cfg.agents.is_empty() {
        let enabled: Vec<String> = match &choices.agents {
            Some(v) => v.clone(),
            None => plan
                .agents
                .iter()
                .filter(|a| a.installed && a.kind != "cloud")
                .map(|a| a.id.clone())
                .collect(),
        };
        for a in workroam_adapters::registry::local() {
            cfg.agents.insert(
                a.id.clone(),
                AgentToggle {
                    state: enabled.contains(&a.id),
                },
            );
        }
    }
    cfg.save(&dir)?;
    out.written.push(".workroam/workroam.toml".into());
    if !McpConfig::path(&dir).exists() {
        McpConfig::default().save(&dir)?;
        out.written.push(".workroam/mcp.toml".into());
    }
    let env_dir = dir.join(".workroam/env");
    std::fs::create_dir_all(&env_dir)?;
    for os in ["windows", "macos", "linux"] {
        let f = env_dir.join(format!("{os}.md"));
        if !f.exists() {
            std::fs::write(&f, format!("<!-- {os}: notes for agents on this system (synced) / 该系统下的补充说明（会同步） -->\n"))?;
            out.written.push(format!(".workroam/env/{os}.md"));
        }
    }
    let skill = dir.join(".agents/skills/workroam/SKILL.md");
    if !skill.exists() {
        workroam_core::paths::write_atomic(&skill, workroam_adapters::SKILL_MD.as_bytes())?;
        out.written.push(".agents/skills/workroam/SKILL.md".into());
    }
    if handoff::ensure(&dir, &plan.project)? {
        out.written.push(handoff::PATH.into());
    }
    let agents_md = dir.join("AGENTS.md");
    let cur = std::fs::read_to_string(&agents_md).unwrap_or_default();
    if let Some(next) = envblock::ensure_sync_block(&cur) {
        workroam_core::paths::write_atomic(&agents_md, next.as_bytes())?;
        out.written.push("AGENTS.md".into());
    }
    // Ignore the machine-local state in git when the project root is a repository.
    let gi = dir.join(".gitignore");
    if dir.join(".git").exists() {
        let s = std::fs::read_to_string(&gi).unwrap_or_default();
        if !s.contains(".workroam/local/") {
            let add = format!(
                "{}# Workroam (machine-local)\n.workroam/local/\n.workroam/SYNCING\n",
                if s.is_empty() || s.ends_with('\n') {
                    ""
                } else {
                    "\n"
                }
            );
            std::fs::write(&gi, format!("{s}{add}"))?;
        }
    }
    let p = Project::at(ws, &plan.project, dir.clone())?;
    match p.generate() {
        Ok(g) => out.warnings.extend(g.warnings),
        Err(e) => out.warnings.push(e.message),
    }
    p.bind()?;
    Ok(out)
}

/// Record a project directory on this device (`project add`, `init`).
pub fn bind(machine: &mut MachineConfig, name: &str, dir: &Path, state: BindingState) {
    machine.projects.insert(
        name.to_string(),
        ProjectBinding {
            path: dir.display().to_string(),
            state,
        },
    );
}

/// `project move`: only while the current directory is empty or missing (exit code 18 otherwise).
pub fn move_dir(machine: &mut MachineConfig, name: &str, new_dir: &Path) -> Result<()> {
    let cur = machine
        .projects
        .get(name)
        .cloned()
        .ok_or_else(|| Error::invalid(format!("project {name} is not on this device")))?;
    let cur_dir = workroam_core::paths::expand_tilde(&cur.path);
    if cur.state == BindingState::Bound
        && !workroam_platform::dircheck::is_empty_or_missing(&cur_dir)
    {
        return Err(Error::new(Code::DirectoryLocked, format!("{name} is already synced into {}", cur_dir.display()))
            .with_hint(format!("remove it on this device (files stay) and add it again: workroam project remove {name} && workroam project add {name} --path <dir>")));
    }
    let issues =
        workroam_platform::dircheck::check_project_dir(new_dir, &other_dirs(machine, name));
    if let Some(i) = issues.iter().find(|i| i.blocking) {
        return Err(Error::invalid(i.message.clone()));
    }
    bind(machine, name, new_dir, BindingState::Pending);
    Ok(())
}

/// `project remove`: forget the binding; files are never touched.
pub fn unbind(machine: &mut MachineConfig, name: &str) -> Option<ProjectBinding> {
    machine.projects.remove(name)
}

/// Refresh Bound / Missing states from the directories (PROJ-09).
pub fn refresh_bindings(ws: &Workspace) -> BTreeMap<String, BindingState> {
    let mut out = BTreeMap::new();
    for (name, b) in &ws.machine.projects {
        let dir = workroam_core::paths::expand_tilde(&b.path);
        let pid = ws.project_id(name);
        let st = match (b.state, dir.exists(), crate::local::Binding::load(&dir)) {
            (BindingState::Archived, _, _) => BindingState::Archived,
            (BindingState::Pending, _, None) => BindingState::Pending,
            (_, false, _) => BindingState::Missing,
            (_, true, Some(bd)) if bd.project_id == pid => BindingState::Bound,
            (_, true, Some(_)) => BindingState::Missing,
            (s, true, None) => {
                if s == BindingState::Bound {
                    BindingState::Missing
                } else {
                    s
                }
            }
        };
        out.insert(name.clone(), st);
    }
    out
}
