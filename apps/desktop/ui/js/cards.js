// Corner cards (UX-01 – UX-05, LEASE-03): approval, progress, finished,
// failed (pinned), arrival and lease conflict. They live in their own
// always-on-top window that never takes keyboard focus.
import { t, has } from "./i18n.js";
import { ipc, act, h, icon, btn, pill, seg, native, onDaemon, size, when, dur, short, runPlans, showError, $, fill } from "./core.js";

const cards = new Map(); // key → element
let box;

export async function startCards(root) {
  document.documentElement.classList.add("cards-window");
  box = h("div", { class: `corner ${navigator.platform.startsWith("Mac") ? "" : "bottom"}` });
  fill(root, box);
  new ResizeObserver(layout).observe(box);
  await onDaemon(onEvent);
  try {
    for (const a of await ipc("approval.list")) approvalCard(a);
    for (const j of await ipc("job.list")) if (j.state === "running" && j.progress) progressCard({ job: j.id, kind: j.kind, project: j.project, progress: j.progress });
  } catch {
    /* the daemon may still be starting; events will fill the cards in */
  }
  layout();
}

function layout() {
  const hgt = cards.size ? Math.ceil(box.getBoundingClientRect().height) : 0;
  native("cards_layout", { height: hgt });
}

function put(key, el, { ttl } = {}) {
  const old = cards.get(key);
  if (old) old.replaceWith(el); else box.prepend(el);
  cards.set(key, el);
  if (ttl) setTimeout(() => drop(key, el), ttl);
  layout();
}

function drop(key, only) {
  const el = cards.get(key);
  if (!el || (only && el !== only)) return;
  el.remove();
  cards.delete(key);
  layout();
}

function card(kind, title, { head, pinned, sicls = "", sic } = {}, ...body) {
  return h("article", { class: `ncard glass ${pinned ? "pinned" : ""}`, "data-kind": kind, role: "status" },
    h("div", { class: "hd" }, h("svg", { class: "lg", "aria-hidden": "true" }, h("use", { href: "#logo" })), head || "Workroam", h("span", { class: "tm" }, when(new Date()))),
    h("h4", {}, sic ? h("span", { class: `si ${sicls}` }, icon(sic)) : null, title), ...body);
}

function onEvent({ event, data }) {
  switch (event) {
    case "approval.requested": return approvalCard(data);
    case "approval.resolved": return drop(`ap:${data.id}`);
    case "job.progress": return progressCard(data);
    case "job.finished": return finishedCard(data);
    case "job.failed": return failedCard(data);
    case "arrival": return arrivalCard(data);
    default:
  }
}

// approval.list: one card per pending request; several stack in the corner.
const approvalCard = act("approval.list", (a) => {
  if (a.kind === "connect") return connectCard(a);
  const d = a.details || {};
  const sm = d.summary || d.plan?.summary || {};
  let skip = false;
  const respond = (decision) => ipc("approval.respond", { id: a.id, decision, skip_ask: decision === "approve" && skip ? "1h" : null }).then(() => drop(`ap:${a.id}`), showError);
  const el = card("approval", a.title, { head: `${a.initiator}${a.remote ? " · " + t("remote") : ""}`, sic: a.kind === "pull" ? "down" : "up", sicls: "acc" },
    h("dl", { class: "meta" }, h("dt", {}, t("project")), h("dd", { class: "mono" }, a.project || "—"),
      h("dt", {}, t("changes")), h("dd", {}, `+${sm.added || 0} ~${sm.modified || 0} -${sm.deleted || 0}${sm.commits ? " · " + t("n_commits", { n: sm.commits }) : ""}${sm.bytes ? " · " + size(sm.bytes) : ""}`)),
    // approval.skip_ask: "don't ask again for this project for an hour".
    h("label", { class: "inline hint" }, h("input", { class: "cb", type: "checkbox", onchange: act("approval.skip_ask", (e) => (skip = e.target.checked)) }), t("skip_ask")),
    h("div", { class: "acts" },
      h("span", { class: "left" }, t("expires_in", { t: dur(Math.max(0, a.timeout_s - (Date.now() - new Date(a.created_at)) / 1000)) })),
      // sync.details: open the details window (files, commits, deny / later / approve).
      btn(t("details"), act("sync.details", () => native("open_window", { label: `details-${a.id}`, route: `details/${a.id}`, title: a.title, width: 560, height: 620 })), { cls: "sm ghost" }),
      btn(t("later"), act("approval.later", () => respond("later")), { cls: "sm" }),
      btn(t("approve"), act("approval.approve", () => respond("approve")), { cls: "sm primary" })));
  put(`ap:${a.id}`, el);
});

// agent.connect_remote: check the confirmation code, pick access and projects.
const connectCard = act("agent.connect_remote", async (a) => {
  let access = "read-only";
  const names = await ipc("project.list").then((l) => l.filter((p) => !p.archived).map((p) => p.name), () => []);
  const picks = new Set(names);
  const el = card("approval", a.title, { head: a.initiator, sic: "link", sicls: "acc" },
    h("p", {}, t("connect.code")), h("div", { class: "codebox big-code" }, a.code || "—"),
    seg([["read-only", t("access.read")], ["sync", t("access.sync")]], access, (v) => (access = v)),
    h("div", { class: "flist" }, names.map((n) => h("label", { class: "inline" }, h("input", { class: "cb", type: "checkbox", checked: true, onchange: (e) => (e.target.checked ? picks.add(n) : picks.delete(n)) }), n))),
    h("div", { class: "acts" },
      btn(t("deny"), () => ipc("approval.respond", { id: a.id, decision: "deny" }).then(() => drop(`ap:${a.id}`), showError), { cls: "sm danger" }),
      btn(t("approve"), () => ipc("approval.respond", { id: a.id, decision: "approve", access, projects: [...picks] }).then(() => drop(`ap:${a.id}`), showError), { cls: "sm primary" })));
  put(`ap:${a.id}`, el);
});

// job.progress: phase, transferred / total, speed, time left, sleep blocked.
const progressCard = act("job.progress", (p) => {
  const pr = p.progress || {};
  const pct = pr.total ? Math.min(100, Math.round((pr.done / pr.total) * 100)) : null;
  const el = card("progress", t(`job.${p.kind}`, { name: p.project }), { sic: p.kind === "pull" ? "down" : "up", sicls: "acc" },
    h("div", { class: "bar" }, h("i", { style: `width:${pct ?? 30}%` })),
    h("dl", { class: "meta" },
      h("dt", {}, has(`phase.${pr.phase}`) ? t(`phase.${pr.phase}`) : pr.phase), h("dd", {}, pr.total ? `${size(pr.done)} / ${size(pr.total)}` : size(pr.done)),
      h("dt", {}, t("speed")), h("dd", {}, `${size(pr.bytes_per_s)}/s · ${t("eta")} ${dur(pr.eta_s)}`)),
    h("div", { class: "acts" }, pr.sleep_blocked ? h("span", { class: "left" }, icon("check", "sm"), t("sleep_blocked")) : null,
      btn(t("cancel"), act("job.cancel", () => ipc("job.cancel", { job: p.job }).then(() => drop(`job:${p.job}`), showError)), { cls: "sm" })));
  put(`job:${p.job}`, el);
});

// job.finished: snapshot id, duration, lease state.
const finishedCard = act("job.finished", (j) => {
  drop(`job:${j.id}`);
  const r = j.result || {};
  const secs = j.started_at && j.finished_at ? (new Date(j.finished_at) - new Date(j.started_at)) / 1000 : null;
  const snap = r.snapshot || r.snapshot_id || r.head;
  const el = card("finished", t(`done.${j.kind}`, { name: j.project }), { sic: "check", sicls: "ok" },
    h("dl", { class: "meta" }, snap ? [h("dt", {}, t("snapshot")), h("dd", { class: "mono" }, short(typeof snap === "string" ? snap : snap.id))] : null,
      h("dt", {}, t("took")), h("dd", {}, dur(secs)),
      r.lease ? [h("dt", {}, t("lease")), h("dd", {}, r.released || r.lease === "released" ? t("lease.released") : t("lease.kept"))] : null),
    h("div", { class: "acts" }, btn(t("close"), () => drop(`done:${j.id}`), { cls: "sm ghost" })));
  put(`done:${j.id}`, el, { ttl: 8000 });
});

// job.failed: stays until dismissed; retry / open checks / ignore.
const failedCard = act("job.failed", (j) => {
  drop(`job:${j.id}`);
  const code = j.error?.code;
  if (code === "approval_timeout" || code === "denied") return;
  if (code === "lease_held") return leaseCard(j);
  const el = card("failed", t(`fail.${j.kind}`, { name: j.project }), { pinned: true, sic: "x", sicls: "bad" },
    h("p", {}, j.error?.message || ""), j.error?.hint ? h("p", { class: "hint" }, j.error.hint) : null,
    h("div", { class: "acts" },
      btn(t("ignore"), () => drop(`fail:${j.id}`), { cls: "sm ghost" }),
      btn(t("open_doctor"), () => native("show_main", { page: "doctor" }), { cls: "sm" }),
      btn(t("retry"), async () => { drop(`fail:${j.id}`); try { await retry(j); } catch (e) { showError(e); } }, { cls: "sm primary" })));
  put(`fail:${j.id}`, el);
});

async function retry(j) {
  const m = { push: "push.plan", pull: "pull.plan", archive: "archive.plan" }[j.kind];
  if (!m) return native("show_main", { page: "projects" });
  await runPlans(await ipc(m, { project: j.project }));
}

// LEASE-03: another device holds the lease.
function leaseCard(j) {
  const holder = j.error?.data?.holder?.name || j.error?.data?.holder || "?";
  const el = card("lease", t("lease.conflict", { name: j.project }), { pinned: true, sic: "lock", sicls: "warn" },
    h("p", {}, t("lease.conflict.d", { holder })),
    h("div", { class: "acts" },
      btn(t("ignore"), () => drop(`lease:${j.id}`), { cls: "sm ghost" }),
      // lease.wait: wait for the other device to release, then pull.
      btn(t("wait_release"), act("lease.wait", async () => { drop(`lease:${j.id}`); await runPlans(await ipc("pull.plan", { project: j.project, wait: true })); }), { cls: "sm" }),
      btn(t("takeover_pull"), act("lease.takeover", async () => { drop(`lease:${j.id}`); await ipc("lease.takeover", { project: j.project }); await runPlans(await ipc("pull.plan", { project: j.project, takeover: true })); }), { cls: "sm danger" })));
  put(`lease:${j.id}`, el);
}

// arrival: another device pushed; later / pull.
const arrivalCard = act("arrival", (a) => {
  const sm = a.summary || {};
  const el = card("arrival", t("arrival.title", { name: a.project }), { head: a.from_device, sic: "down", sicls: "acc" },
    h("dl", { class: "meta" }, h("dt", {}, t("from")), h("dd", {}, `${a.from_device} · ${when(a.time)}`),
      h("dt", {}, t("changes")), h("dd", {}, `+${sm.added || 0} ~${sm.modified || 0} -${sm.deleted || 0}${sm.commits ? " · " + t("n_commits", { n: sm.commits }) : ""}`)),
    a.touches_local ? h("p", { class: "warnline" }, icon("warn", "sm"), t("arrival.conflict", { n: a.local_changes })) : null,
    h("div", { class: "acts" },
      btn(t("later"), () => drop(`arr:${a.project}`), { cls: "sm" }),
      btn(t("pull"), act("pull", async () => { drop(`arr:${a.project}`); await runPlans(await ipc("pull.plan", { project: a.project })); }), { cls: "sm primary" })));
  put(`arr:${a.project}`, el);
});

// The approval details window (opened from "view details").
export async function startDetails(root, id) {
  let a;
  try {
    a = await ipc("approval.list", { id });
  } catch (e) {
    fill(root, h("div", { class: "pad" }, h("p", {}, e.message)));
    return;
  }
  const d = a.details || {};
  const plan = d.plan || d;
  const changes = plan.changes || [];
  const commits = (plan.repos || []).flatMap((r) => (r.commits || []).map((c) => ({ repo: r.name, ...c })));
  const respond = (decision) => ipc("approval.respond", { id, decision }).then(() => window.__TAURI__?.window.getCurrentWindow().close(), showError);
  fill(root, h("div", { class: "details" },
    h("h2", {}, a.title), h("p", { class: "hint" }, `${a.initiator}${a.project ? " · " + a.project : ""} · ${when(a.created_at)}`),
    plan.lease ? h("p", { class: "hint" }, t("lease"), ": ", plan.lease.state === "mine" ? t("lease.mine") : plan.lease.holder?.name || plan.lease.state) : null,
    h("h4", { class: "cap" }, t("changed_files", { n: changes.length })),
    h("div", { class: "group scroll-list tall" }, changes.length ? changes.map((c) => h("div", { class: "flrow" }, h("span", { class: `k ${c.kind[0]}` }, { added: "+", modified: "~", deleted: "-" }[c.kind] || "~"), h("span", { class: "pth" }, c.path), h("span", { class: "sz" }, c.size != null ? size(c.size) : ""))) : h("p", { class: "hint pad" }, t("no_changes"))),
    commits.length ? [h("h4", { class: "cap" }, t("commits")), h("div", { class: "group" }, commits.map((c) => h("div", { class: "flrow" }, icon("git", "sm"), h("span", { class: "pth" }, `${c.repo} ${short(c.id || c.sha)} ${c.subject || c.message || ""}`), h("span", { class: "sz" }, ""))))] : null,
    h("div", { class: "acts" },
      btn(t("deny"), act("approval.deny", () => respond("deny")), { cls: "danger" }),
      btn(t("later"), () => respond("later")),
      btn(t("approve"), () => respond("approve"), { cls: "primary" }))));
}
