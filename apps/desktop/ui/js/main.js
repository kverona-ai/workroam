// The main window: projects, history, archives, devices, checks and
// settings. Every button is one IPC call (or a plan + job.start), the same
// calls the matching `workroam` command makes (docs/cli-ui-parity.md).
import { t } from "./i18n.js";
import {
  ipc, act, h, icon, btn, pill, sw, seg, field, input, callout, toast, showError, sheet, acts, ask, askTyped,
  pickDir, pickFile, saveFile, copy, openUrl, reveal, size, when, dur, short, runPlans, onDaemon, listen, $, native, fill } from "./core.js";
import { checkRow } from "./setup.js";
import { updateTray } from "./tray.js";

const S = { page: "projects", sel: null, status: null, list: [], info: null, histProject: null, snap: null, archSearch: "" };
const PAGES = [
  ["projects", "folder", "nav.projects"],
  ["history", "clock", "nav.history"],
  ["archives", "archive", "nav.archives"],
  ["devices", "devices", "nav.devices"],
  ["doctor", "shield", "nav.doctor"],
  ["settings", "sliders", "nav.settings"],
];
let side, toolbar, panel, banner;

export async function startMain(root, opts = {}) {
  side = h("nav", { class: "side", "aria-label": "Workroam" });
  toolbar = h("div", { class: "toolbar" });
  panel = h("div", { class: "panel" });
  banner = h("div", { class: "banner", hidden: true });
  fill(root, h("div", { class: "win app-win" }, banner, h("div", { class: "win-body appbody" }, side, h("div", { class: "app-main" }, toolbar, panel))));
  await refresh();
  if (opts.firstProject) initProject();
  if (opts.pull?.length) pullAll(opts.pull.map((p) => p[0]));
  let pending;
  onDaemon((ev) => {
    if (ev.event === "daemon.down") { banner.hidden = false; fill(banner, icon("warn"), t("daemon_down")); return; }
    if (ev.event === "daemon.up") banner.hidden = true;
    if (["status.changed", "job.finished", "job.failed", "arrival", "daemon.up", "approval.requested", "approval.resolved"].includes(ev.event)) {
      clearTimeout(pending);
      pending = setTimeout(() => refresh().catch(() => {}), 300);
    }
    if (ev.event === "job.progress") updateTray(S, ev.data);
  });
  listen("navigate", (p) => go(p));
  setInterval(() => refresh().catch(() => {}), 60000);
}

export function go(page) {
  S.page = page;
  draw();
}

// home.summary: sidebar footer (storage, workspace, this device) and the tray header.
const refresh = act("home.summary", async () => {
  const [status, list, info] = await Promise.all([ipc("status", { all: true }), ipc("project.list"), ipc("daemon.info")]);
  S.status = status;
  S.list = list;
  S.info = info;
  updateTray(S);
  draw();
});

function draw() {
  drawSide();
  const pages = { projects, history, archives, devices, doctor, settings };
  (pages[S.page] || projects)();
}

function drawSide() {
  const bound = S.list.filter((p) => !p.archived);
  const count = { projects: bound.length, archives: S.list.filter((p) => p.archived).length || null, devices: null };
  const warn = (S.status?.projects || []).some((p) => p.last_failure || !p.dir_exists);
  fill(side, ...[
    h("div", { class: "grp" }, t("grp.workspace")),
    PAGES.slice(0, 3).map(([id, ic, k]) => item(id, ic, k, id === "projects" && warn ? h("span", { class: "wdot" }) : count[id] != null ? h("span", { class: "cnt" }, count[id]) : null)),
    h("div", { class: "grp" }, t("grp.device")),
    PAGES.slice(3).map(([id, ic, k]) => item(id, ic, k)),
    h("div", { class: "foot" },
      h("div", { class: "row1" }, h("span", { class: "dot", style: S.info?.paused ? "color:var(--warn)" : "" }), h("b", {}, S.status?.device?.name || "—"), S.status?.device?.access === "read-only" ? h("span", { class: "tag" }, t("read_only")) : null),
      h("span", { class: "mono hint", title: S.status?.workspace }, S.status?.workspace || ""),
      S.info?.paused ? h("span", { class: "hint" }, t("paused_until", { time: when(S.info.paused_until) })) : null)].flat());
}

function item(id, ic, k, extra) {
  return h("button", { class: `it ${S.page === id ? "on" : ""}`, type: "button", onclick: () => go(id), "aria-current": S.page === id ? "page" : null }, icon(ic), t(k), extra);
}

function bar(title, ...right) {
  fill(toolbar, h("h2", {}, title), ...right);
}

const statusOf = (name) => (S.status?.projects || []).find((p) => p.project === name);

// ------------------------------------------------------------- projects --

// project.list: the project list in the main window (and the tray).
const projects = act("project.list", () => {
  const list = S.list.filter((p) => !p.archived);
  bar(t("nav.projects"), btn(t("init_project"), () => initProject(), { cls: "primary", ic: "plus" }));
  if (!list.length) {
    fill(panel, h("div", { class: "empty" }, h("svg", { class: "logo-lg", "aria-hidden": "true" }, h("use", { href: "#logo" })), h("h3", {}, t("no_projects")), h("p", { class: "hint" }, t("no_projects.d")), btn(t("init_project"), () => initProject(), { cls: "primary", ic: "plus" })));
    return;
  }
  if (!list.find((p) => p.name === S.sel)) S.sel = (list.find((p) => p.on_this_device) || list[0]).name;
  const rows = list.map((p) => {
    const st = statusOf(p.name);
    const [cls, label] = projectState(p, st);
    return h("button", { class: `prow ${p.name === S.sel ? "on" : ""}`, type: "button", onclick: () => { S.sel = p.name; projects(); } },
      h("span", { class: "top" }, h("span", { class: "nm" }, p.name), h("span", { class: `state ${cls}` }, h("span", { class: "dot" }), label)),
      h("span", { class: "meta" }, p.head ? `${p.head.device_name} · ${when(p.head.created_at)}` : t("never_synced")));
  });
  const detail = h("div", { class: "detail" });
  fill(panel, h("div", { class: "split" }, h("div", { class: "plist" }, rows), detail));
  projectDetail(detail, list.find((p) => p.name === S.sel));
});

function projectState(p, st) {
  if (p.binding === "not_added" || !p.on_this_device) return ["dim", t("st.not_added")];
  if (!st) return ["dim", "—"];
  if (!st.dir_exists) return ["bad", t("st.dir_missing")];
  if (st.syncing || st.job_running) return ["acc", t("st.syncing")];
  if (st.last_failure) return ["bad", t("st.failed")];
  if (st.incoming) return ["acc", t("st.incoming")];
  const lc = st.local_changes || {};
  if (lc.added + lc.modified + lc.deleted + lc.commits > 0) return ["warn", t("st.to_push")];
  if (st.lease?.state === "other") return ["amb", t("st.leased_by", { name: st.lease.holder?.name })];
  return ["ok", t("st.synced")];
}

function tile(name) {
  let x = 0;
  for (const c of name) x = (x * 31 + c.charCodeAt(0)) % 360;
  return h("span", { class: "ptile", style: `--h1:hsl(${x} 85% 62%);--h2:hsl(${(x + 40) % 360} 80% 48%)` }, name.slice(0, 2).toUpperCase());
}

// project.status: lease, latest snapshot, changes waiting, repositories.
const projectDetail = act("project.status", async (el, p) => {
  const st = statusOf(p.name);
  const hd = h("div", { class: "hd" }, h("div", { class: "who" }, tile(p.name), h("div", {}, h("h3", {}, p.name),
    h("p", {}, st?.dir ? h("span", { class: "lockdir", title: st.dir }, icon("folder", "sm"), h("span", { class: "ell" }, st.dir)) : t("st.not_added")))));
  const body = [hd];
  if (!p.on_this_device) {
    body.push(addHere(p));
    fill(el, ...body);
    return;
  }
  const lc = st.local_changes || {};
  const pending = lc.added + lc.modified + lc.deleted;
  hd.append(h("div", { class: "acts-row" },
    btn(t("push"), () => push(p.name), { cls: "primary", ic: "up", disabled: !st.dir_exists }),
    btn(t("push_release"), () => pushRelease(p.name), { ic: "up", disabled: !st.dir_exists }),
    btn(t("pull_dots"), () => pull(p.name), { ic: "down", disabled: !st.dir_exists }),
    moreMenu(p, st)));
  if (!st.dir_exists) body.push(dirMissing(p, st));
  if (st.lease?.state === "other") body.push(callout("warn", "lock", h("div", {}, h("b", {}, t("lease.other", { name: st.lease.holder?.name || "?" })), h("div", { class: "hint" }, t("lease.other.d", { time: when(st.lease.renewed_at) }))), btn(t("takeover_pull"), () => takeover(p.name), { cls: "sm" })));
  if (st.last_failure) body.push(callout("bad", "warn", h("div", {}, h("b", {}, t("last_failed")), h("div", { class: "hint" }, st.last_failure.error?.message || ""))));
  body.push(relay(p, st, pending));
  body.push(h("div", { class: "sect" }, h("h4", {}, t("sync")), projectSettings(p.name)));
  if (st.repos?.length) {
    body.push(h("div", { class: "sect" }, h("h4", {}, t("repos")), h("div", { class: "group" }, st.repos.map((r) => h("div", { class: "repo" }, icon("git"), h("span", { class: "nm" }, r.name),
      pill(t(`mode.${r.mode}`), "acc"), r.history && r.history !== "full" ? pill(r.history) : null,
      r.in_progress ? pill(r.in_progress, "warn", "warn") : null, !r.can_push ? pill(t("cannot_push"), "warn") : null,
      h("span", { class: "end" }, r.branch || "", r.dirty ? " · " + t("dirty") : ""))))));
  }
  const ho = h("div", { class: "handoff" }, h("span", { class: "hint" }, t("loading")));
  body.push(h("div", { class: "sect" }, h("h4", {}, t("handoff")), ho));
  fill(el, ...body);
  handoff(ho, p.name);
});

function relay(p, st, pending) {
  const me = S.status?.device?.name;
  const holder = st.lease?.holder?.name;
  const other = holder && holder !== me ? holder : p.head && p.head.device_name !== me ? p.head.device_name : t("relay.other");
  const node = (cls, ic, b, s, lease, mine) => h("div", { class: `rnode ${cls} ${lease ? "lease" : ""}` }, h("span", { class: "rd" }, icon(ic, "lg")), h("b", {}, b), mine ? h("span", { class: "me" }, t("this_device")) : null, h("small", {}, s));
  const lease = st.lease?.state;
  const paused = S.info?.paused;
  const l1 = h("span", { class: `rlink l1 ${paused ? "off" : pending ? "fr" : ""}` }, h("span", { class: "ln" }), h("span", { class: "lb" }, paused ? t("paused") : pending ? t("n_files", { n: pending }) : ""));
  const l2 = h("span", { class: `rlink l2 ${paused ? "off" : st.incoming ? "fl" : ""}` }, h("span", { class: "ln" }), h("span", { class: "lb" }, st.incoming ? t("incoming") : ""));
  const stat = (v, u, label) => h("div", {}, h("b", {}, v, u ? h("small", {}, u) : null), h("span", {}, label));
  const [num, unit] = size(lc(st).bytes).split(" ");
  return h("div", { class: "relay" },
    h("div", { class: "track" },
      node("co", "laptop", me || "—", st.last_push ? t("pushed_ago", { time: when(st.last_push) }) : t("never_pushed"), lease === "mine", true),
      node("st", "bucket", t("relay.storage"), p.head ? `${short(p.head.snapshot)} · ${when(p.head.created_at)}` : "—", lease === "free" || !lease, false),
      node("ho", "desktop", other, holder && holder !== me ? t("holds_lease") : "", lease === "other", false),
      l1, l2),
    h("div", { class: "stats" }, stat(pending, "", t("stat.files")), stat(num, unit, t("stat.size")), stat(lc(st).commits, "", t("stat.commits")), stat(when(st.last_pull), "", t("stat.last_pull"))));
}
const lc = (st) => st.local_changes || { bytes: 0, commits: 0 };

function moreMenu(p, st) {
  const items = [
    [t("archive_dots"), () => archiveProject(p.name)],
    [t("reveal"), () => reveal(st.dir)],
    !st.last_push && !st.last_pull ? [t("move_dots"), () => moveProject(p.name, st.dir)] : null,
    [t("remove_here_dots"), () => removeHere(p.name, st.dir)],
  ].filter(Boolean);
  return menuButton(items);
}

export function menuButton(items) {
  const menu = h("div", { class: "popmenu glass", hidden: true }, items.map(([label, f]) => h("button", { class: "mi", type: "button", onclick: () => { menu.hidden = true; f(); } }, label)));
  const b = btn("", (e) => { e.stopPropagation(); menu.hidden = !menu.hidden; }, { cls: "icon", ic: "more", title: t("more") });
  document.addEventListener("click", () => (menu.hidden = true));
  return h("span", { class: "popwrap" }, b, menu);
}

// project.handoff: the "handoff" section of a project.
const handoff = act("project.handoff", async (el, name) => {
  const r = await ipc("handoff.read", { project: name });
  const ho = r.handoff;
  if (!ho?.exists || ho.empty) {
    fill(el, h("span", { class: "hint" }, t("handoff.empty", { path: r.path || "HANDOFF.md" })));
    return;
  }
  fill(el, 
    h("span", { class: "h" }, [ho.agent, ho.device, ho.time ? when(ho.time) : null].filter(Boolean).join(" · ") || r.path),
    ...ho.sections.filter((s) => s.text || s.items?.length).map((s) => h("div", {}, h("b", {}, s.heading), s.items?.length ? h("ul", {}, s.items.map((i) => h("li", {}, i))) : h("p", {}, s.text))));
});

// Per-project overrides: auto sync, agent approval, remote connections.
function projectSettings(name) {
  const g = h("div", { class: "setgrp" }, h("div", { class: "setrow" }, h("span", { class: "hint" }, t("loading"))));
  ipc("config.list", { project: name }).then((list) => {
    const v = Object.fromEntries(list.map((x) => [x.key, x]));
    fill(g, 
      setrow(t("auto_sync"), v["sync.auto"]?.source === "project" ? t("project_override") : t("inherits"), seg([["ask", t("auto.ask")], ["auto", t("auto.auto")], ["off", t("auto.off")]], v["sync.auto"]?.value, act("settings.sync_auto", (x) => setConfig("sync.auto", x, name)))),
      setrow(t("agent_approval"), t("agent_approval.d"), sw(v["sync.agent_requires_approval"]?.value, act("settings.agent_approval", (x) => setConfig("sync.agent_requires_approval", x, name)), t("agent_approval"))),
      setrow(t("remote_allowed"), t("remote_allowed.d"), sw(v["remote.enabled"]?.value, act("settings.remote", (x) => setConfig("remote.enabled", x, name)), t("remote_allowed"))));
  }, showError);
  return g;
}

function setrow(title, sub, ctl) {
  return h("div", { class: "setrow" }, h("div", { class: "tx" }, h("b", {}, title), sub ? h("span", {}, sub) : null), ctl);
}

async function setConfig(key, value, project) {
  try {
    await ipc("config.set", { key, value, project: project || null });
    toast(t("saved"), "ok");
  } catch (e) {
    showError(e);
  }
}

// project.add: a workspace project not on this computer yet.
function addHere(p) {
  const root = S.status ? (S.list.find((x) => x.dir)?.dir || "").replace(/[\\/][^\\/]+$/, "") : "";
  const dir = input({ mono: true, value: root ? `${root}/${p.name}` : "" });
  const choose = btn(t("choose"), async () => { const d = await pickDir(p.name, dir.value); if (d) dir.value = d; }, { cls: "sm", ic: "folder" });
  const add = act("project.add", async () => {
    await ipc("project.add", { project: p.name, path: dir.value.trim() });
    const plans = await ipc("pull.plan", { project: p.name });
    await runPlans(plans);
    toast(t("adding", { name: p.name }), "ok");
    await refresh();
  });
  return h("div", { class: "sect" }, callout("info", "down", t("not_added.d", { device: p.head?.device_name || "?" })),
    field(t("local_dir"), h("div", { class: "inrow" }, dir, choose)), h("div", { class: "acts-row" }, btn(t("add_pull"), add, { cls: "primary", ic: "down" })));
}

function dirMissing(p, st) {
  return callout("bad", "folder", h("div", {}, h("b", {}, t("dir_missing", { dir: st.dir })), h("div", { class: "hint" }, t("dir_missing.d"))),
    h("div", { class: "acts-row" },
      btn(t("recheck_dir"), act("project.recheck", async () => { await refresh(); if (!statusOf(p.name)?.dir_exists) toast(t("still_missing"), "warn"); }), { cls: "sm", ic: "refresh" }),
      btn(t("relocate_dots"), act("project.relocate", async () => {
        const d = await pickDir(t("relocate_dots"));
        if (!d) return;
        await ipc("project.remove", { project: p.name });
        await ipc("project.add", { project: p.name, path: d });
        await runPlans(await ipc("pull.plan", { project: p.name }));
        await refresh();
      }), { cls: "sm" })));
}

// push: plan, then start the job; progress shows on the corner cards.
export const push = act("push", async (name, extra = {}) => {
  await runPlans(await ipc("push.plan", { project: name, ...extra }));
});
const pushRelease = act("push.release", (name) => push(name, { release: true }));

// pull: confirm when it would touch local changes, as `workroam pull` does.
export const pull = act("pull", async (name, extra = {}) => {
  const plans = await ipc("pull.plan", { project: name, ...extra });
  for (const pl of plans) {
    if (pl.plan?.up_to_date && !extra.takeover) { toast(t("up_to_date", { name: pl.project }), "ok"); return; }
    if (pl.plan?.touches_local && !(await ask(t("pull_title", { name: pl.project }), t("touches_local", { n: pl.plan.local_changes })))) return;
  }
  await runPlans(plans);
});

export const pullAll = (names) => Promise.all(names.map((n) => pull(n)));

export const takeover = act("lease.takeover", async (name) => {
  if (!(await ask(t("takeover.title"), t("takeover.d"), { ok: t("takeover_pull"), danger: true }))) return;
  await ipc("lease.takeover", { project: name });
  await pull(name, { takeover: true });
});

const moveProject = act("project.move", async (name, dir) => {
  const d = await pickDir(t("move_dots"), dir);
  if (!d) return;
  await ipc("project.move", { project: name, path: d });
  await refresh();
});

const removeHere = act("project.remove", async (name, dir) => {
  if (!(await ask(t("remove.title", { name }), t("remove.d", { dir }), { ok: t("remove"), danger: true }))) return;
  await ipc("project.remove", { project: name });
  toast(t("removed", { name }), "ok");
  await refresh();
});

// project.init: the "add project" dialog (choose a folder, scan, repo modes, imports, push).
export const initProject = act("project.init", async () => {
  const dir = await pickDir(t("init.choose"));
  if (!dir) return;
  // project.scan: live checks shown in the dialog.
  const scan = await act("project.scan", () => ipc("project.scan", { dir }))();
  if (!scan) return;
  await sheet(t("init.title"), (close) => {
    const name = input({ mono: true, value: scan.project });
    const repoCfg = {};
    const skip = new Set();
    const issues = (scan.dir_issues || []).map((i) => callout(i.blocking ? "bad" : "warn", "warn", i.message));
    // project.repo_modes: change each repository's mode and history depth.
    const repoRow = act("project.repo_modes", (r) => {
      const cfg = (repoCfg[r.name] = { ...r.recommended, path: r.path });
      const mode = h("select", { class: "input sel", onchange: (e) => (cfg.mode = e.target.value) }, ["bundle", "remote", "files"].map((m) => h("option", { value: m, selected: m === cfg.mode }, t(`mode.${m}`))));
      const hist = h("select", { class: "input sel", onchange: (e) => (cfg.history = e.target.value) }, [["full", t("hist.full")], ["recent:50", t("hist.recent", { n: 50 })], ["snapshot", t("hist.snapshot")]].map(([v, l]) => h("option", { value: v, selected: v === (cfg.history || "full") }, l)));
      const inc = h("input", { class: "cb", type: "checkbox", checked: true, onchange: (e) => (e.target.checked ? skip.delete(r.name) : skip.add(r.name)) });
      return h("div", { class: "repo" }, inc, icon("git"), h("span", { class: "nm" }, r.path || r.name), h("span", { class: "hint grow" }, r.reason || ""), mode, hist);
    });
    const imports = scan.imports?.items || [];
    const take = h("input", { class: "cb", type: "checkbox" });
    const blocking = (scan.dir_issues || []).some((i) => i.blocking);
    return [
      field(t("project_name"), name), h("p", { class: "hint mono" }, scan.dir), ...issues,
      scan.repos?.length ? h("div", { class: "sect" }, h("h4", {}, t("repos_found")), h("div", { class: "group" }, scan.repos.map(repoRow))) : null,
      imports.length ? h("div", { class: "sect" }, h("h4", {}, t("imports")), h("div", { class: "flist" }, imports.map((i) => h("div", {}, `${i.kind} · ${i.source}`))), h("label", { class: "inline" }, take, t("take_imported"))) : null,
      (scan.problems || []).length ? h("div", { class: "sect" }, h("h4", {}, t("xplat")), scan.problems.map((p) => callout("warn", "warn", p.message))) : null,
      acts(btn(t("cancel"), () => close(null)), btn(t("init_push"), async () => {
        try {
          const done = await ipc("project.init", { dir, name: name.value.trim() || null, choices: { repos: repoCfg, skip_repos: [...skip], take_imported: take.checked, agents: null } });
          close(done);
          S.sel = done.project;
          await runPlans(await ipc("push.plan", { project: done.project }));
          await refresh();
        } catch (e) {
          showError(e);
        }
      }, { cls: "primary", ic: "up", disabled: blocking })),
    ];
  }, { wide: true });
});

// archive.create: note, keep full history, unsynced changes are pushed first.
export const archiveProject = act("archive.create", async (name) => {
  if (!name) {
    const names = S.list.filter((p) => !p.archived).map((p) => p.name);
    name = await sheet(t("archive_dots"), (close) => {
      const sel = h("select", { class: "input" }, names.map((n) => h("option", { value: n }, n)));
      return [field(t("project"), sel), acts(btn(t("cancel"), () => close(null)), btn(t("continue"), () => close(sel.value), { cls: "primary" }))];
    });
    if (!name) return;
  }
  const dry = await ipc("archive.plan", { project: name, dry_run: true });
  const pl = dry.plan;
  const ok = await sheet(t("archive.title", { name }), (close) => {
    const note = input({ placeholder: t("archive.note") });
    const keep = h("input", { class: "cb", type: "checkbox" });
    return [
      h("p", { class: "dim" }, t("archive.d", { files: pl.files, size: size(pl.size) })),
      pl.unsynced ? callout("warn", "up", t("archive.unsynced", { n: pl.unsynced })) : null,
      pl.blocker ? callout("bad", "warn", pl.blocker.message || String(pl.blocker)) : null,
      field(t("note"), note), h("label", { class: "inline" }, keep, t("keep_history")),
      acts(btn(t("cancel"), () => close(null)), btn(pl.unsynced ? t("push_archive") : t("archive"), () => close({ note: note.value, keep_history: keep.checked }), { cls: "primary", disabled: !!pl.blocker })),
    ];
  });
  if (!ok) return;
  await runPlans(await ipc("archive.plan", { project: name, ...ok }));
  toast(t("archiving", { name }), "ok");
});

// ---------------------------------------------------------------- history --

// history.list: timeline with source device, file and commit counts, note.
const history = act("history.list", async () => {
  const names = S.list.filter((p) => p.on_this_device || p.archived).map((p) => p.name);
  S.histProject = names.includes(S.histProject) ? S.histProject : S.sel && names.includes(S.sel) ? S.sel : names[0];
  const sel = h("select", { class: "input sel", onchange: (e) => { S.histProject = e.target.value; S.snap = null; history(); } }, names.map((n) => h("option", { value: n, selected: n === S.histProject }, n)));
  bar(t("nav.history"), sel, btn(t("restore_project_dots"), () => restoreProject(S.histProject), { ic: "restore", disabled: !S.histProject }));
  if (!S.histProject) { fill(panel, h("p", { class: "hint" }, t("no_projects"))); return; }
  const list = await ipc("history.list", { project: S.histProject, limit: 100 });
  const me = S.status?.device?.name;
  const tl = h("div", { class: "timeline" }, list.map((s) => h("button", { class: `snap ${S.snap === s.id ? "on" : ""}`, type: "button", onclick: () => { S.snap = s.id; history(); } },
    h("span", { class: `dev ${s.device === me ? "" : "w"}` }, icon(s.device === me ? "laptop" : "desktop")),
    h("span", {}, h("span", { class: "l1" }, s.short_id, h("span", { class: "hint" }, s.device), s.archived ? pill(t("archived")) : null, s.restored_from ? pill(t("restored")) : null),
      h("span", { class: "l2" }, when(s.time), " · ", diffText(s.summary), s.note ? ` · ${s.note}` : "")))));
  const right = h("div", { class: "detail" });
  fill(panel, h("div", { class: "split" }, tl, right));
  if (S.snap) showSnapshot(right, S.histProject, S.snap);
  else right.append(h("p", { class: "hint" }, t("pick_snapshot")));
});

function diffText(sm = {}) {
  return h("span", { class: "diff" }, h("span", { class: "a" }, `+${sm.added || 0}`), " ", h("span", { class: "m" }, `~${sm.modified || 0}`), " ", h("span", { class: "d" }, `-${sm.deleted || 0}`), sm.commits ? ` · ${t("n_commits", { n: sm.commits })}` : "");
}

// history.show: the files a snapshot changed, with checkboxes.
const showSnapshot = act("history.show", async (el, project, id) => {
  const r = await ipc("history.show", { project, snapshot: id });
  const picked = new Set();
  const rows = r.changes.map((c) => h("label", { class: "flrow" }, h("input", { class: "cb", type: "checkbox", onchange: (e) => (e.target.checked ? picked.add(c.path) : picked.delete(c.path)) }),
    h("span", { class: "pth" }, h("span", { class: `k ${c.kind[0]}` }, { added: "+", modified: "~", deleted: "-" }[c.kind] || "~"), " ", c.path), h("span", { class: "sz" }, c.size != null ? size(c.size) : "")));
  fill(el, h("div", { class: "hd" }, h("div", {}, h("h3", {}, r.snapshot.short_id), h("p", {}, `${r.snapshot.device} · ${when(r.snapshot.time)}${r.snapshot.note ? " · " + r.snapshot.note : ""}`))),
    h("div", { class: "group scroll-list tall" }, rows.length ? rows : h("p", { class: "hint pad" }, t("no_changes"))),
    h("div", { class: "acts-row" },
      btn(t("restore_selected"), act("restore.files", async () => restoreFiles(project, id, [...picked], null)), { cls: "primary", ic: "restore" }),
      btn(t("export_to_dots"), act("restore.export", async () => {
        const to = await pickDir(t("export_to_dots"));
        if (to) await restoreFiles(project, id, [...picked], to);
      }), { ic: "export" })));
});

async function restoreFiles(project, snapshot, paths, to) {
  if (!to && !paths.length) return toast(t("pick_files"), "warn");
  const dry = await ipc("restore.plan", { project, snapshot, paths, to, dry_run: true });
  const pl = dry.plan;
  const msg = t("restore.confirm", { n: paths.length || pl.files, dir: to || pl.dir }) + (pl.saved_local?.length ? " " + t("restore.saved_local", { n: pl.saved_local.length }) : "");
  if (!(await ask(t("restore"), msg, { ok: t("restore") }))) return;
  await runPlans(await ipc("restore.plan", { project, snapshot, paths, to }));
}

// restore.project: the whole project; the dialog names the conflict-copy location.
const restoreProject = act("restore.project", async (project) => {
  const snap = S.snap || (await ipc("history.list", { project, limit: 1 }))[0]?.id;
  if (!snap) return;
  const dry = await ipc("restore.plan", { project, snapshot: snap, dry_run: true });
  const pl = dry.plan;
  if (!(await ask(t("restore_project_dots"), t("restore.project_d", { snap: short(snap), dir: pl.dir, n: pl.saved_local?.length || 0 }), { ok: t("restore"), danger: true }))) return;
  await runPlans(await ipc("restore.plan", { project, snapshot: snap }));
});

// --------------------------------------------------------------- archives --

// archive.list: list and search.
const archives = act("archive.list", async () => {
  const search = input({ placeholder: t("search"), value: S.archSearch, onkeydown: (e) => { if (e.key === "Enter") { S.archSearch = e.target.value; archives(); } } });
  bar(t("nav.archives"), h("label", { class: "search" }, icon("search"), search),
    btn(t("archive_dots"), () => archiveProject(), { ic: "archive" }),
    btn(t("import_pkg"), () => importPackage(), { ic: "down" }));
  const list = await ipc("archive.list", { search: S.archSearch || null });
  if (!list.length) { fill(panel, h("div", { class: "empty" }, icon("archive", "lg"), h("p", { class: "hint" }, t("no_archives")))); return; }
  fill(panel, h("div", { class: "table" }, list.map((a) => h("div", { class: "trow" },
    h("div", {}, h("div", { class: "nm" }, a.project), h("div", { class: "sm" }, [a.note, a.keep_history ? t("keep_history") : null].filter(Boolean).join(" · "))),
    h("div", { class: "sm hide-n" }, `${a.device || a.device_name || ""} · ${when(a.archived_at || a.time)}`),
    h("div", { class: "num hide-n" }, size(a.size)),
    h("div", { class: "acts-row" },
      btn(t("restore_dots"), () => restoreArchive(a.project), { cls: "sm primary" }),
      menuButton([[t("extract_dots"), () => extractArchive(a.project)], [t("export_pkg"), () => exportPackage(a.project)], [t("delete_archive"), () => deleteArchive(a.project)]]))))));
});

// archive.restore_plan: the detected scenario ①–⑦ and what will happen.
const restoreArchive = act("archive.restore_plan", async (project, to = null) => {
  const r = await ipc("archive.restore_plan", { project, to, dry_run: true });
  const pl = r.plan;
  const marks = ["", "①", "②", "③", "④", "⑤", "⑥", "⑦"];
  const choice = await sheet(t("restore_archive", { name: project }), (close) => {
    let prefer = pl.scenario === "local_newer" ? "local" : pl.scenario === "diverged" ? "merge" : null;
    // archive.prefer: ③ keep local / archive wins; ⑤ merge / archive wins / restore elsewhere to compare.
    const opts = act("archive.prefer", () => pl.scenario === "local_newer"
      ? seg([["local", t("prefer.local")], ["archive", t("prefer.archive")]], prefer, (v) => (prefer = v))
      : pl.scenario === "diverged" ? seg([["merge", t("prefer.merge")], ["archive", t("prefer.archive")], ["compare", t("prefer.compare")]], prefer, (v) => (prefer = v)) : null)();
    return [
      h("div", { class: "scen" }, h("div", { class: "ttl" }, h("span", { class: "big-mark" }, marks[pl.scenario_number] || ""), t(`scen.${pl.scenario}`)), h("p", { class: "hint" }, pl.reason || t(`scen.${pl.scenario}.d`)), h("p", { class: "mono hint" }, pl.dir)),
      pl.differing?.length ? h("div", { class: "flist scroll-list" }, pl.differing.slice(0, 200).map((f) => h("div", {}, f.path || f))) : null,
      opts,
      acts(btn(t("other_dir"), act("archive.other_dir", async () => { const d = await pickDir(t("other_dir")); if (d) close({ to: d, again: true }); })),
        btn(t("cancel"), () => close(null)),
        btn(t("restore"), async () => {
          let compare_to = null;
          if (prefer === "compare") { compare_to = await pickDir(t("prefer.compare")); if (!compare_to) return; }
          close({ prefer, compare_to });
        }, { cls: "primary", disabled: pl.scenario === "occupied" })),
    ];
  });
  if (!choice) return;
  if (choice.again) return restoreArchive(project, choice.to);
  await runPlans(await ipc("archive.restore_plan", { project, to, prefer: choice.prefer, compare_to: choice.compare_to }));
  toast(t("restoring", { name: project }), "ok");
});

const extractArchive = act("archive.extract", async (project) => {
  const to = await pickDir(t("extract_dots"));
  if (!to) return;
  const r = await ipc("archive.extract", { project, to });
  toast(t("extracted", { dir: r.dir || to }), "ok");
});

const deleteArchive = act("archive.delete", async (project) => {
  const typed = await askTyped(t("delete_archive"), t("delete_archive.d", { name: project }), project);
  if (!typed) return;
  await ipc("archive.delete", { project, confirm: typed });
  toast(t("deleted"), "ok");
  archives();
});

// archive.package: export / import a .wrarchive file.
const exportPackage = act("archive.package", async (project) => {
  const file = await saveFile(t("export_pkg"), `${project}.wrarchive`, [{ name: "Workroam archive", extensions: ["wrarchive"] }]);
  if (!file) return;
  const pw = await askPassword(t("pkg.password"));
  if (pw == null) return;
  await ipc("archive.export", { project, file, password: pw || null });
  toast(t("exported"), "ok");
});

const importPackage = act("archive.package", async () => {
  const file = await pickFile(t("import_pkg"), [{ name: "Workroam archive", extensions: ["wrarchive"] }]);
  if (!file) return;
  const pw = await askPassword(t("pkg.password"));
  if (pw == null) return;
  await ipc("archive.import", { file, password: pw || null });
  toast(t("imported"), "ok");
  archives();
});

function askPassword(title) {
  return sheet(title, (close) => {
    const pw = input({ type: "password", placeholder: t("pkg.password.ph") });
    return [field(t("password"), pw, t("pkg.password.d")), acts(btn(t("cancel"), () => close(null)), btn(t("ok"), () => close(pw.value), { cls: "primary" }))];
  });
}

// ---------------------------------------------------------------- devices --

// device.list: name, OS, access, reachable hosts, last sync, leases.
const devices = act("device.list", async () => {
  bar(t("nav.devices"), btn(t("add_device"), () => addDevice(), { cls: "primary", ic: "plus" }));
  const list = await ipc("device.list");
  fill(panel, h("div", { class: "devlist" }, list.map((d) => h("div", { class: "devrow" },
    h("span", { class: "di" }, icon(d.os === "macos" ? "laptop" : "desktop")),
    h("div", {}, h("div", { class: "nm" }, d.name, d.this_device ? h("span", { class: "me" }, t("this_device")) : null, d.access === "read-only" ? pill(t("read_only")) : null),
      h("div", { class: "sm" }, h("span", {}, d.os), h("span", {}, t("last_sync", { time: when(d.last_sync) })), d.reachable?.length ? h("span", { class: "mono" }, d.reachable.join(", ")) : null,
        d.leases?.length ? h("span", {}, icon("lock", "sm"), " ", d.leases.join(", ")) : null)),
    d.this_device ? h("span") : btn(t("remove"), () => removeDevice(d), { cls: "sm danger" })))));
});

// device.add: a pairing code and QR code, optionally for a read-only device.
const addDevice = act("device.add", async () => {
  await sheet(t("add_device"), (close) => {
    const ro = h("input", { class: "cb", type: "checkbox" });
    const out = h("div");
    const gen = async () => {
      fill(out, h("span", { class: "spin" }));
      try {
        const r = await ipc("device.pairing_code", { read_only: ro.checked });
        fill(out, h("div", { class: "pairbox" }, r.qr_svg || r.pairing_qr_svg ? h("div", { class: "qr", html: r.qr_svg || r.pairing_qr_svg }) : null,
          h("div", { class: "grow" }, h("p", { class: "hint" }, t("pairing.howto")), h("div", { class: "codebox wrap" }, r.pairing_code || r.code), btn(t("copy"), () => copy(r.pairing_code || r.code), { cls: "sm", ic: "copy" }),
            r.short_code ? h("p", {}, t("short_code"), " ", h("b", { class: "mono big-code" }, r.short_code)) : null)));
      } catch (e) {
        fill(out, callout("bad", "warn", e.message));
      }
    };
    ro.addEventListener("change", gen);
    gen();
    return [h("label", { class: "inline" }, ro, t("read_only_device")), out, acts(btn(t("done"), () => close(true), { cls: "primary" }))];
  });
});

// device.remove: "remove and rotate the key".
const removeDevice = act("device.remove", async (d) => {
  const r = await sheet(t("remove_device", { name: d.name }), (close) => {
    const ak = input({ mono: true });
    const sk = input({ mono: true, type: "password" });
    return [h("p", { class: "dim" }, t("remove_device.d")), h("div", { class: "grid2" }, field(t("new_access_key"), ak), field(t("new_secret_key"), sk)), h("p", { class: "hint" }, t("remove_device.hint")),
      acts(btn(t("cancel"), () => close(null)), btn(t("remove_rotate"), () => close({ ak: ak.value.trim() || null, sk: sk.value || null }), { cls: "danger solid" }))];
  });
  if (!r) return;
  await ipc("device.remove", { device: d.id, new_access_key: r.ak, new_secret_key: r.sk });
  toast(t("device_removed", { name: d.name }), "ok");
  devices();
});

// ----------------------------------------------------------------- doctor --

const doctor = act("doctor", async () => {
  bar(t("nav.doctor"), btn(t("verify_dots"), () => verify(), { ic: "shield" }), btn(t("rerun"), () => runDoctor(), { cls: "primary", ic: "refresh" }));
  if (!S.doctor) return runDoctor();
  drawDoctor();
});

async function runDoctor() {
  fill(panel, h("div", { class: "acts-row" }, h("span", { class: "spin" }), t("checks.running")));
  try {
    S.doctor = await ipc("doctor.run");
  } catch (e) {
    fill(panel, callout("bad", "warn", e.message));
    return;
  }
  drawDoctor();
}

function drawDoctor() {
  const r = S.doctor;
  const groups = [...new Set(r.results.map((x) => x.group))];
  fill(panel, h("div", { class: "settings" }, h("p", { class: "hint" }, t("ran_at", { time: when(r.ran_at) })),
    groups.map((g) => h("div", {}, h("h4", {}, t(`group.${g}`)), h("div", { class: "checks" }, r.results.filter((x) => x.group === g).map(checkRow))))));
}

// verify: check the data, primary storage or the mirror, with a sample ratio.
const verify = act("verify", async () => {
  const o = await sheet(t("verify_dots"), (close) => {
    let target = "primary";
    const pct = h("input", { type: "range", min: 0, max: 100, step: 5, value: 10 });
    const lbl = h("span", { class: "num" }, "10%");
    pct.addEventListener("input", () => (lbl.textContent = `${pct.value}%`));
    return [h("p", { class: "dim" }, t("verify.d")), seg([["primary", t("verify.primary")], ["backup", t("verify.backup")]], target, (v) => (target = v)),
      setrow(t("verify.sample"), t("verify.sample.d"), h("span", { class: "inrow" }, pct, lbl)),
      acts(btn(t("cancel"), () => close(null)), btn(t("verify"), () => close({ backup: target === "backup", read_data: Number(pct.value) }), { cls: "primary" }))];
  });
  if (!o) return;
  toast(t("verifying"));
  const r = await ipc("verify.run", o);
  await ask(t("verify"), t("verify.ok", { n: r.packs_checked ?? r.checked ?? "?" }));
});

// --------------------------------------------------------------- settings --

// settings.list: the settings page.
const settings = act("settings.list", async () => {
  bar(t("nav.settings"));
  const [list, agents, backup] = await Promise.all([ipc("config.list"), ipc("agent.list"), ipc("backup.configure", { action: "list" }).catch(() => null)]);
  const v = Object.fromEntries(list.map((x) => [x.key, x.value]));
  const sec = (title, ...rows) => h("div", {}, h("h4", {}, title), h("div", { class: "setgrp" }, ...rows));
  const textSetting = (key, value, ph, actId) => {
    const i = input({ value: Array.isArray(value) ? value.join(", ") : value ?? "", placeholder: ph, mono: true });
    i.addEventListener("change", act(actId, () => setConfig(key, i.value)));
    return i;
  };
  const idle = input({ type: "number", min: 1, value: v["sync.idle_minutes"] || 15, class: "input num-in" });
  idle.addEventListener("change", () => setConfig("sync.idle_minutes", idle.value));
  const ntfy = input({ mono: true, value: v["notify.ntfy"] || "", placeholder: "https://ntfy.sh/<topic>" });
  ntfy.addEventListener("change", act("settings.notify", () => setConfig("notify.ntfy", ntfy.value)));
  fill(panel, h("div", { class: "settings" },
    // settings.machine
    sec(t("set.machine"),
      setrow(t("device_name"), null, textSetting("device.name", v["device.name"], "", "settings.machine")),
      setrow(t("root"), t("root.d"), textSetting("root", v.root, "", "settings.machine")),
      setrow(t("reachable"), t("reachable.hint"), textSetting("reachable", v.reachable, "git.corp.example", "settings.machine")),
      setrow(t("autostart"), t("autostart.d"), sw(v.autostart, act("settings.autostart", async (x) => { await setConfig("autostart", x); native(`plugin:autostart|${x ? "enable" : "disable"}`).catch(() => {}); }), t("autostart"))),
      setrow(t("language"), null, seg([["", t("lang.auto")], ["zh", "中文"], ["en", "English"]], v["ui.lang"] || "", async (x) => { await setConfig("ui.lang", x); location.reload(); }))),
    sec(t("auto_sync"),
      setrow(t("auto_sync"), t("auto_sync.d"), seg([["ask", t("auto.ask")], ["auto", t("auto.auto")], ["off", t("auto.off")]], v["sync.auto"], act("settings.sync_auto", (x) => setConfig("sync.auto", x)))),
      setrow(t("idle"), t("idle.d"), h("span", { class: "inrow" }, sw(v["sync.idle_minutes"] > 0, act("settings.idle", (x) => setConfig("sync.idle_minutes", x ? idle.value || 15 : 0)), t("idle")), idle, h("span", { class: "hint" }, t("minutes")))),
      setrow(t("on_sleep"), t("on_sleep.d"), sw(v["sync.on_sleep"] !== "off", act("settings.on_sleep", (x) => setConfig("sync.on_sleep", x ? "ask" : "off")), t("on_sleep"))),
      setrow(t("on_wake"), t("on_wake.d"), sw(v["sync.on_wake"], act("settings.on_wake", (x) => setConfig("sync.on_wake", x)), t("on_wake"))),
      setrow(t("agent_approval"), t("agent_approval.d"), sw(v["sync.agent_requires_approval"], act("settings.agent_approval", (x) => setConfig("sync.agent_requires_approval", x)), t("agent_approval")))),
    sec(t("set.notify"),
      setrow(t("ntfy"), t("ntfy.d"), ntfy),
      setrow(t("on_success"), t("on_success.d"), sw(v["notify.on_success"], act("settings.notify", (x) => setConfig("notify.on_success", x)), t("on_success")))),
    agentSection(agents, v),
    sec(t("set.storage"),
      setrow(t("workspace"), S.status?.workspace, btn(t("update_key_dots"), () => updateCredentials(), { cls: "sm", ic: "key" })),
      setrow(t("retention"), t("retention.d"), h("span", { class: "inrow" }, retention("retention.days", v["retention.days"], t("days")), retention("retention.weekly", v["retention.weekly"], t("weeks")), btn(t("prune_now"), act("settings.retention", async () => { const r = await ipc("prune.run", {}); toast(t("pruned", { n: r.removed ?? r.forgotten ?? 0 }), "ok"); }), { cls: "sm" })))),
    sec(t("set.encryption"),
      setrow(t("change_pw"), t("change_pw.d"), btn(t("change_pw_dots"), () => changePassword(), { cls: "sm" })),
      setrow(t("reset_pw"), t("reset_pw.d"), btn(t("reset_pw_dots"), () => resetPassword(), { cls: "sm" }))),
    backupSection(backup)));
});

function retention(key, value, unit) {
  const i = input({ type: "number", min: 0, value: value ?? "", class: "input num-in" });
  i.addEventListener("change", act("settings.retention", () => setConfig(key, i.value)));
  return h("span", { class: "inrow" }, i, h("span", { class: "hint" }, unit));
}

// agent.list: version, adapter version, match; connect local and cloud agents.
const agentSection = act("agent.list", (r, v) => {
  const enabledIn = r.enabled_in || {};
  const rows = r.agents.map((a) => {
    const d = a.detected;
    const state = d.installed ? (d.supported === false ? pill(t("unsupported"), "warn") : pill(`${d.version || ""}`, "ok")) : pill(t("not_installed"));
    const ctl = a.cloud
      ? btn(t("connect_x", { name: d.name }), () => connectRemote(a.detected.id, d.name), { cls: "sm", ic: "link" })
      : h("span", { class: "inrow" },
        a.mcp ? h("span", { class: "inrow hint" }, "MCP", sw(v["mcp.enabled"] !== false, act("settings.mcp", (x) => setConfig("mcp.enabled", x)), "MCP")) : null,
        btn(enabledIn[d.id]?.length ? t("connected_in", { n: enabledIn[d.id].length }) : t("connect"), () => connectLocal(d.id), { cls: "sm", disabled: !d.installed }));
    return setrow(d.name, `${a.cloud ? t("cloud_agent") : t("local_agent")} · ${t("adapter_v", { v: d.adapter_version })}${d.range ? " · " + d.range : ""}`, h("span", { class: "inrow" }, state, ctl));
  });
  const conns = (r.connections || []).map((c) => setrow(`${c.agent} · ${c.access}`, `${(c.projects || []).join(", ") || t("all_projects")} · ${when(c.created_at)}`,
    h("span", { class: "inrow" }, btn("webhook", () => webhook(c.agent), { cls: "sm" }), btn(t("revoke"), () => disconnect(c.agent), { cls: "sm danger" }))));
  const devSel = h("select", { class: "input sel", onchange: act("settings.remote", (e) => setConfig("remote.default_device", e.target.value)) }, h("option", { value: "" }, t("this_device")));
  ipc("device.list").then((ds) => ds.forEach((d) => devSel.append(h("option", { value: d.name, selected: v["remote.default_device"] === d.name }, d.name))), () => {});
  return h("div", {}, h("h4", {}, t("set.agents")), h("div", { class: "setgrp" }, ...rows, ...conns,
    setrow(t("mcp_secrets"), t("mcp_secrets.d"), secretChips()),
    setrow(t("default_device"), r.relay?.connected ? t("relay_on", { url: r.relay.public_url || r.relay.url || "" }) : t("relay_off"), devSel)));
});

// secret.set: the `${secret:name}` values of .workroam/mcp.toml live in this
// computer's keychain (`workroam secret set / remove / list`).
async function secretRefs() {
  const names = new Map();
  for (const p of S.status?.projects || []) {
    const r = await ipc("secret.list", { project: p.project }).catch(() => null);
    for (const s of r?.secrets || []) {
      const x = names.get(s.name) || { name: s.name, set: !!s.set, where: [] };
      x.set = x.set || !!s.set;
      x.where.push(`${p.project}: ${(s.servers || []).join(", ")}`);
      names.set(s.name, x);
    }
  }
  return [...names.values()];
}

function secretChips() {
  const box = h("span", { class: "inrow", style: "flex-wrap:wrap" }, btn(t("secret_set_dots"), () => mcpSecret(), { cls: "sm", ic: "key" }));
  secretRefs().then((refs) => fill(box,
    ...refs.map((s) => { const p = pill(s.set ? s.name : t("secret_unset", { name: s.name }), s.set ? "ok" : ""); p.title = s.where.join("\n"); return p; }),
    btn(t("secret_set_dots"), () => mcpSecret(refs), { cls: "sm", ic: "key" })), () => {});
  return box;
}

const mcpSecret = act("secret.set", async (refs) => {
  refs = refs || (await secretRefs().catch(() => []));
  const r = await sheet(t("mcp_secrets"), (close) => {
    const name = input({ mono: true, list: "secret-names", placeholder: "jira_token" });
    const value = input({ mono: true, type: "password" });
    return [h("p", { class: "hint" }, t("mcp_secrets.d")), h("datalist", { id: "secret-names" }, ...refs.map((s) => h("option", { value: s.name }))),
      field(t("secret_name"), name), field(t("secret_value"), value),
      acts(btn(t("cancel"), () => close(null)), btn(t("delete"), () => close({ name: name.value.trim(), remove: true }), { cls: "danger" }),
        btn(t("save"), () => close({ name: name.value.trim(), value: value.value }), { cls: "primary" }))];
  });
  if (!r || !r.name) return;
  if (r.remove) {
    if (!(await ask(t("delete"), t("secret_remove.d", { name: r.name }), { ok: t("delete"), danger: true }))) return;
    await ipc("secret.remove", { name: r.name });
    toast(t("removed", { name: r.name }), "ok");
  } else {
    await ipc("secret.set", { name: r.name, value: r.value });
    toast(t("secret_saved", { name: r.name }), "ok");
  }
  settings();
});

const connectLocal = act("agent.connect_local", async (agent) => {
  const r = await ipc("agent.connect", { agent });
  toast(t("agent_connected", { name: agent, n: r.projects?.length ?? r.enabled?.length ?? "" }), "ok");
  settings();
});

// agent.connect_remote: open the agent's connector page or copy the setup text.
const connectRemote = act("agent.connect_remote", async (agent, name) => {
  let r = await ipc("agent.connect", { agent });
  await sheet(t("connect_x", { name }), (close) => {
    const relayUrl = input({ mono: true, placeholder: "https://relay.example.com" });
    const body = h("div", { class: "grid1" });
    const drawR = () => fill(body, 
      r.instructions ? h("pre", { class: "codebox pre" }, r.instructions) : null,
      r.url ? h("div", { class: "codebox wrap" }, r.url) : null,
      r.bind_code ? h("p", {}, t("bind_code"), " ", h("b", { class: "mono big-code" }, r.bind_code)) : null,
      h("div", { class: "acts-row" }, r.connector_url ? btn(t("open_connector"), () => openUrl(r.connector_url), { cls: "sm", ic: "export" }) : null,
        r.url ? btn(t("copy_url"), () => copy(r.url), { cls: "sm", ic: "copy" }) : null,
        r.instructions ? btn(t("copy_setup"), () => copy(r.instructions), { cls: "sm", ic: "copy" }) : null),
      !r.url ? field(t("relay_url"), h("div", { class: "inrow" }, relayUrl, btn(t("bind"), async () => { try { r = await ipc("agent.connect", { agent, relay: relayUrl.value.trim() }); drawR(); } catch (e) { showError(e); } }, { cls: "sm" })), t("relay_url.d")) : null,
      callout("info", "shield", t("connect.approve_hint")));
    drawR();
    return [body, acts(btn(t("done"), () => close(true), { cls: "primary" }))];
  }, { wide: true });
});

const disconnect = act("agent.disconnect", async (agent) => {
  if (!(await ask(t("revoke"), t("revoke.d", { name: agent }), { ok: t("revoke"), danger: true }))) return;
  await ipc("agent.disconnect", { agent });
  settings();
});

const webhook = act("agent.webhook", async (agent) => {
  const r = await sheet("Webhook", (close) => {
    const url = input({ mono: true, placeholder: "https://…" });
    const secret = input({ mono: true, type: "password" });
    return [h("p", { class: "hint" }, t("webhook.d")), field("URL", url), field(t("secret_opt"), secret), acts(btn(t("cancel"), () => close(null)), btn(t("save"), () => close({ url: url.value.trim(), secret: secret.value || null }), { cls: "primary" }))];
  });
  if (!r) return;
  await ipc("agent.webhook", { agent, ...r });
  toast(t("saved"), "ok");
});

const updateCredentials = act("credentials.update", async () => {
  const r = await sheet(t("update_key_dots"), (close) => {
    const ak = input({ mono: true });
    const sk = input({ mono: true, type: "password" });
    return [h("p", { class: "hint" }, t("update_key.d")), field("Access Key ID", ak), field("Secret Access Key", sk), acts(btn(t("cancel"), () => close(null)), btn(t("save"), () => close({ access_key: ak.value.trim(), secret_key: sk.value }), { cls: "primary" }))];
  });
  if (!r) return;
  await ipc("credentials.update", r);
  toast(t("saved"), "ok");
});

const changePassword = act("password.change", async () => {
  const r = await sheet(t("change_pw_dots"), (close) => {
    const old = input({ type: "password" });
    const a = input({ type: "password" });
    const b = input({ type: "password" });
    return [field(t("old_pw"), old), field(t("new_pw"), a), field(t("password_again"), b),
      acts(btn(t("cancel"), () => close(null)), btn(t("save"), () => (a.value !== b.value ? toast(t("pw.mismatch"), "bad") : close({ old: old.value, new: a.value })), { cls: "primary" }))];
  });
  if (!r) return;
  await ipc("password.change", r);
  toast(t("pw_changed"), "ok");
});

const resetPassword = act("password.reset", async () => {
  const r = await sheet(t("reset_pw_dots"), (close) => {
    const rk = input({ mono: true, placeholder: "XXXX-XXXX-…" });
    const a = input({ type: "password" });
    const b = input({ type: "password" });
    return [field(t("recovery_key"), rk), field(t("new_pw"), a), field(t("password_again"), b),
      acts(btn(t("cancel"), () => close(null)), btn(t("save"), () => (a.value !== b.value ? toast(t("pw.mismatch"), "bad") : close({ recovery_key: rk.value, new: a.value })), { cls: "primary" }))];
  });
  if (!r) return;
  await ipc("password.reset", r);
  toast(t("pw_changed"), "ok");
});

// backup: the off-site mirror, its last result, "back up now", remove.
const backupSection = act("backup", (b) => {
  const cfg = b?.backup || b?.config || (b && b.kind ? b : null);
  const rows = cfg ? [
    setrow(t("mirror"), `${cfg.kind || ""} ${cfg.endpoint || cfg.path || ""}${cfg.bucket ? " / " + cfg.bucket : ""}`, h("span", { class: "inrow" },
      btn(t("backup_now"), async () => { toast(t("backing_up")); try { const r = await ipc("backup.run"); toast(t("backup_done", { n: r.snapshots ?? r.copied ?? "" }), "ok"); settings(); } catch (e) { showError(e); } }, { cls: "sm primary" }),
      btn(t("remove"), async () => { if (await ask(t("remove_mirror"), t("remove_mirror.d"), { danger: true, ok: t("remove") })) { await ipc("backup.configure", { action: "remove" }); settings(); } }, { cls: "sm danger" }))),
    setrow(t("last_backup"), b.last ? `${when(b.last.time || b.last.finished_at)} · ${b.last.ok === false ? t("failed") : t("ok")}` : t("never"), h("span")),
  ] : [setrow(t("mirror"), t("mirror.none"), btn(t("add_mirror"), () => addMirror(), { cls: "sm", ic: "plus" }))];
  return h("div", {}, h("h4", {}, t("set.backup")), h("div", { class: "setgrp" }, ...rows));
});

async function addMirror() {
  const r = await sheet(t("add_mirror"), (close) => {
    let kind = "s3";
    const ep = input({ mono: true, placeholder: "https://s3.backup.example" });
    const bucket = input({ mono: true });
    const prefix = input({ mono: true });
    const ak = input({ mono: true });
    const sk = input({ mono: true, type: "password" });
    const path = input({ mono: true, placeholder: "/Volumes/nas/workroam" });
    const s3 = h("div", { class: "grid1" }, field(t("endpoint"), ep), h("div", { class: "grid2" }, field(t("bucket"), bucket), field(t("prefix"), prefix)), h("div", { class: "grid2" }, field("Access Key ID", ak), field("Secret Access Key", sk)));
    const loc = field(t("local_dir"), path);
    loc.hidden = true;
    const sched = h("select", { class: "input sel" }, [["daily", t("daily")], ["weekly", t("weekly")], ["manual", t("manual")]].map(([x, l]) => h("option", { value: x }, l)));
    return [seg([["s3", "S3"], ["local", t("local_dir")]], kind, (x) => { kind = x; s3.hidden = x !== "s3"; loc.hidden = x === "s3"; }), s3, loc, field(t("schedule"), sched),
      acts(btn(t("cancel"), () => close(null)), btn(t("save"), () => close(kind === "s3" ? { kind, endpoint: ep.value.trim(), bucket: bucket.value.trim(), prefix: prefix.value.trim(), access_key: ak.value.trim(), secret_key: sk.value, schedule: sched.value } : { kind, path: path.value.trim(), schedule: sched.value }), { cls: "primary" }))];
  });
  if (!r) return;
  await ipc("backup.configure", { action: "add", ...r });
  settings();
}

export { S, refresh };
