// The tray menu: built here from the daemon state, drawn by Rust
// (src-tauri/src/tray.rs); clicks come back as `tray` events.
import { t } from "./i18n.js";
import { ipc, act, native, listen, runPlans, toast } from "./core.js";

let state;
let progress = new Map();

export function updateTray(S, prog) {
  state = S;
  if (prog) {
    progress.set(prog.job, prog);
    clearTimeout(updateTray.t);
    updateTray.t = setTimeout(() => draw(), 1000);
    return;
  }
  draw();
}

function draw() {
  const S = state;
  if (!S?.status) return;
  const ps = S.status.projects || [];
  const syncing = ps.some((p) => p.syncing || p.job_running);
  const approvals = S.status.daemon?.approvals || 0;
  const failed = ps.filter((p) => p.last_failure);
  const items = [
    { kind: "text", text: `Workroam · ${S.status.device?.name || ""}` },
    { kind: "text", text: S.info?.paused ? t("tray.paused") : syncing ? t("tray.syncing") : t("tray.n_projects", { n: ps.length }) },
    { kind: "sep" },
  ];
  if (approvals) items.push({ kind: "item", id: "approvals", text: t("tray.approvals", { n: approvals }) });
  for (const f of failed) items.push({ kind: "item", id: `failed:${f.project}`, text: `⚠ ${t("tray.failed", { name: f.project })}` });
  if (approvals || failed.length) items.push({ kind: "sep" });
  for (const p of ps) {
    const lc = p.local_changes || {};
    const n = lc.added + lc.modified + lc.deleted;
    items.push({
      kind: "submenu",
      text: `${p.project}${n ? `  ↑${n}` : ""}${p.incoming ? "  ↓" : ""}`,
      items: [
        { kind: "item", id: `push:${p.project}`, text: t("push") },
        { kind: "item", id: `pull:${p.project}`, text: t("pull") },
        { kind: "item", id: `open:${p.project}`, text: t("tray.details") },
      ],
    });
  }
  items.push({ kind: "sep" },
    { kind: "item", id: "push_all_release", text: t("tray.push_all_release"), enabled: ps.length > 0 },
    { kind: "item", id: "pull_all", text: t("tray.pull_all"), enabled: ps.length > 0 },
    { kind: "item", id: "pause", text: S.info?.paused ? t("tray.resume") : t("tray.pause") },
    { kind: "sep" },
    { kind: "item", id: "doctor", text: t("tray.doctor") });
  const st = approvals || failed.length ? "attention" : syncing ? "syncing" : "idle";
  native("tray_update", { model: { state: st, tooltip: `Workroam · ${S.info?.paused ? t("tray.paused") : syncing ? t("tray.syncing") : t("tray.idle")}`, items } }).catch(() => {});
}

const projects = () => (state?.status?.projects || []).map((p) => p.project);

// push.all_release: push every project and release its lease (leaving this computer).
const pushAllRelease = act("push.all_release", async () => {
  await runPlans(await ipc("push.plan", { all: true, release: true }));
  toast(t("tray.pushing_all"), "ok");
});

// pull.all: "pull latest" for every project on this computer.
const pullAll = act("pull.all", async () => {
  await runPlans(await ipc("pull.plan", { all: true }));
});

// sync.pause: pause automatic sync for an hour, or resume it.
const pause = act("sync.pause", async () => {
  if (state?.info?.paused) await ipc("sync.resume");
  else await ipc("sync.pause", { for: "1h" });
});

export function trayActions({ push, pull, go, select }) {
  listen("tray", async (id) => {
    const [k, name] = id.split(/:(.*)/s);
    switch (k) {
      case "push": return push(name);
      case "pull": return pull(name);
      case "open": select(name); return native("show_main", { page: "projects" });
      case "failed": select(name); return native("show_main", { page: "projects" });
      case "approvals": return native("cards_layout", { height: 1 });
      case "push_all_release": return pushAllRelease();
      case "pull_all": return pullAll();
      case "pause": return pause();
      case "doctor": go("doctor"); return native("show_main", { page: "doctor" });
      default:
    }
  });
  return projects;
}
