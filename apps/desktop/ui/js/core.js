// Shared plumbing for every window: the daemon bridge (Tauri command, or
// dev/serve.mjs in a plain browser), DOM helpers, dialogs and formatting.
import { t } from "./i18n.js";

const T = window.__TAURI__;
export const inTauri = !!T;

// ------------------------------------------------------------------ IPC --

export class WrError extends Error {
  constructor(e) {
    super(e?.message || String(e));
    this.code = e?.code || "internal";
    this.hint = e?.hint || null;
    this.data = e?.data ?? null;
  }
}

/** Call a workroamd method (architecture §15); same calls as the CLI. */
export async function ipc(method, params = {}) {
  if (T) {
    try {
      return await T.core.invoke("ipc", { method, params });
    } catch (e) {
      throw new WrError(typeof e === "string" ? { message: e } : e);
    }
  }
  const r = await fetch("/ipc", { method: "POST", body: JSON.stringify({ method, params }) });
  const v = await r.json();
  if (v.err) throw new WrError(v.err);
  return v.ok;
}

/** Daemon events: {event, data}; also daemon.up / daemon.down. */
export function onDaemon(fn) {
  if (T) return T.event.listen("daemon", (e) => fn(e.payload));
  const es = new EventSource("/events");
  es.onmessage = (m) => fn(JSON.parse(m.data));
  es.onopen = () => fn({ event: "daemon.up", data: {} });
  es.onerror = () => fn({ event: "daemon.down", data: {} });
  return Promise.resolve(() => es.close());
}

/** App-level commands (windows, tray, files). No-ops in a plain browser. */
export async function native(cmd, args = {}) {
  if (T) return T.core.invoke(cmd, args);
  if (cmd === "decode_qr") return null;
  return null;
}

export function listen(name, fn) {
  if (T) return T.event.listen(name, (e) => fn(e.payload));
  return Promise.resolve(() => {});
}

export async function pickDir(title, defaultPath) {
  if (T?.dialog) return T.dialog.open({ directory: true, title, defaultPath });
  return window.prompt(title, defaultPath || "") || null;
}

export async function pickFile(title, filters, defaultPath) {
  if (T?.dialog) return T.dialog.open({ directory: false, title, filters, defaultPath });
  return window.prompt(title, defaultPath || "") || null;
}

export async function saveFile(title, defaultPath, filters) {
  if (T?.dialog) return T.dialog.save({ title, defaultPath, filters });
  return window.prompt(title, defaultPath || "") || null;
}

export async function openUrl(url) {
  if (T?.opener) return T.opener.openUrl(url);
  window.open(url, "_blank", "noopener");
}

export async function reveal(path) {
  if (T?.opener) return T.opener.revealItemInDir(path);
}

export async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
    toast(t("copied"), "ok");
  } catch {
    toast(t("copy_failed"), "bad");
  }
}

export async function notify(title, body) {
  const n = T?.notification;
  if (!n) return;
  try {
    let ok = await n.isPermissionGranted();
    if (!ok) ok = (await n.requestPermission()) === "granted";
    if (ok) n.sendNotification({ title, body });
  } catch {
    /* notifications are a convenience */
  }
}

// --------------------------------------------------------------- actions --

/**
 * Every user action in the app is wrapped by act(<parity id>, fn): the ids
 * are the desktop column of docs/cli-ui-parity.md, and tests/parity.rs
 * checks that each id in apps/desktop/actions.json is wired here.
 */
export const actions = new Set();
export function act(id, fn) {
  actions.add(id);
  return async (...args) => {
    try {
      return await fn(...args);
    } catch (e) {
      showError(e);
    }
  };
}

// ------------------------------------------------------------------- DOM --

export const $ = (s, el = document) => el.querySelector(s);
export const $$ = (s, el = document) => [...el.querySelectorAll(s)];

export function h(tag, attrs = {}, ...kids) {
  const e = document.createElementNS(tag === "svg" || tag === "use" ? "http://www.w3.org/2000/svg" : "http://www.w3.org/1999/xhtml", tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v == null) continue;
    if (k === "class") e.setAttribute("class", v);
    else if (k.startsWith("on") && typeof v === "function") e.addEventListener(k.slice(2), v);
    else if (k === "html") e.innerHTML = v;
    else if (k === "value" && "value" in e) e.value = v;
    else if (k === "checked" || k === "disabled" || k === "selected" || k === "hidden") e[k] = !!v;
    else e.setAttribute(k, v === true ? "" : v);
  }
  for (const k of kids.flat(Infinity)) if (k != null && k !== false) e.append(k.nodeType ? k : String(k));
  return e;
}

/** replaceChildren that skips null / false and flattens arrays. */
export function fill(el, ...kids) {
  el.replaceChildren(...kids.flat(Infinity).filter((k) => k != null && k !== false));
  return el;
}

export function icon(name, cls = "") {
  const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  s.setAttribute("class", `ic ${cls}`.trim());
  s.setAttribute("aria-hidden", "true");
  const u = document.createElementNS("http://www.w3.org/2000/svg", "use");
  u.setAttribute("href", `#i-${name}`);
  s.append(u);
  return s;
}

export function btn(label, onclick, { cls = "", ic, title, disabled, type = "button" } = {}) {
  return h("button", { class: `btn ${cls}`.trim(), type, onclick, title, disabled }, ic ? icon(ic, "sm") : null, label);
}

export function pill(text, kind = "", ic) {
  return h("span", { class: `pill ${kind}`.trim() }, ic ? icon(ic) : null, text);
}

export function sw(checked, onchange, label) {
  return h("input", { class: "sw", type: "checkbox", checked, onchange: (e) => onchange(e.target.checked, e), "aria-label": label });
}

export function seg(options, value, onchange) {
  const el = h("div", { class: "seg", role: "radiogroup" });
  for (const [v, label] of options) {
    el.append(h("button", { type: "button", role: "radio", class: v === value ? "on" : "", "aria-checked": String(v === value), onclick: () => {
      for (const b of el.children) { b.classList.remove("on"); b.setAttribute("aria-checked", "false"); }
      const b = [...el.children].find((x) => x.dataset.v === v);
      b.classList.add("on");
      b.setAttribute("aria-checked", "true");
      onchange(v);
    }, "data-v": v }, label));
  }
  return el;
}

export function field(label, input, hint) {
  return h("div", { class: "field" }, h("label", { class: "lbl" }, label), input, hint ? h("div", { class: "hint" }, hint) : null);
}

export function input(attrs = {}) {
  return h("input", { class: `input ${attrs.mono ? "mono" : ""}`.trim(), spellcheck: "false", autocomplete: "off", ...attrs, mono: null });
}

export function callout(kind, ic, ...body) {
  return h("div", { class: `callout ${kind}` }, icon(ic), h("span", {}, ...body));
}

// ----------------------------------------------------- toasts and sheets --

let toastBox;
export function toast(msg, kind = "") {
  toastBox ||= document.body.appendChild(h("div", { class: "toasts", role: "status", "aria-live": "polite" }));
  const el = h("div", { class: `toast glass ${kind}` }, msg);
  toastBox.append(el);
  setTimeout(() => el.classList.add("out"), 3600);
  setTimeout(() => el.remove(), 4000);
}

export function showError(e) {
  console.error(e);
  const msg = e?.message || String(e);
  toast(e?.hint ? `${msg} — ${e.hint}` : msg, "bad");
}

/**
 * A sheet (modal inside the window). `build(close)` returns the body; the
 * promise resolves with whatever close() gets.
 */
export function sheet(title, build, { wide } = {}) {
  return new Promise((resolve) => {
    const host = $(".win") || document.body;
    let wrap;
    const close = (v) => {
      wrap.remove();
      resolve(v);
    };
    const s = h("div", { class: `sheet glass ${wide ? "wide" : ""}`, role: "dialog", "aria-modal": "true", "aria-label": title }, h("h3", {}, title));
    wrap = h("div", { class: "sheet-wrap", onkeydown: (e) => e.key === "Escape" && close(null) }, s);
    s.append(...[build(close)].flat());
    host.append(wrap);
    setTimeout(() => (s.querySelector("input,textarea,select,button.primary") || s).focus?.(), 30);
  });
}

export function acts(...buttons) {
  return h("div", { class: "acts" }, ...buttons);
}

/** Yes / no question in a sheet. */
export function ask(title, body, { ok = t("ok"), danger } = {}) {
  return sheet(title, (close) => [
    typeof body === "string" ? h("p", { class: "dim" }, body) : body,
    acts(btn(t("cancel"), () => close(false)), btn(ok, () => close(true), { cls: danger ? "danger solid" : "primary" })),
  ]);
}

/** Ask the person to type a name to confirm a destructive step. */
export function askTyped(title, body, name, { ok } = {}) {
  return sheet(title, (close) => {
    const inp = input({ mono: true, placeholder: name });
    const go = btn(ok || t("delete"), () => close(inp.value), { cls: "danger solid", disabled: true });
    inp.addEventListener("input", () => (go.disabled = inp.value !== name));
    return [h("p", { class: "dim" }, body), field(t("type_to_confirm", { name }), inp), acts(btn(t("cancel"), () => close(null)), go)];
  });
}

// ------------------------------------------------------------ formatting --

export function size(n) {
  if (n == null || isNaN(n)) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${i ? n.toFixed(n < 10 ? 1 : 0) : n} ${u[i]}`;
}

export function when(ts) {
  if (!ts) return "—";
  const d = new Date(ts);
  if (isNaN(d)) return "—";
  const s = (Date.now() - d) / 1000;
  if (s < 45) return t("just_now");
  if (s < 3600) return t("min_ago", { n: Math.round(s / 60) });
  if (s < 86400) return t("h_ago", { n: Math.round(s / 3600) });
  return d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

export function dur(sec) {
  if (sec == null) return "—";
  sec = Math.round(sec);
  if (sec < 60) return `${sec} s`;
  if (sec < 3600) return `${Math.floor(sec / 60)} min ${sec % 60} s`;
  return `${Math.floor(sec / 3600)} h ${Math.floor((sec % 3600) / 60)} min`;
}

export const short = (id) => (id || "").slice(0, 8);

/** Pointer highlight on glass (DESIGN.md §9). */
export function glassPointer(root = document) {
  root.addEventListener("pointermove", (e) => {
    const g = e.target.closest?.(".glass");
    if (!g) return;
    const r = g.getBoundingClientRect();
    g.style.setProperty("--mx", `${e.clientX - r.left}px`);
    g.style.setProperty("--my", `${e.clientY - r.top}px`);
    g.classList.add("lit");
  });
  root.addEventListener("pointerout", (e) => e.target.closest?.(".glass")?.classList.remove("lit"));
}

/** Run planned jobs (push.plan / pull.plan / … results) and return results. */
export async function runPlans(plans, { wait = false } = {}) {
  const list = Array.isArray(plans) ? plans : [plans];
  const out = [];
  for (const pl of list) {
    if (pl.blocker) throw new WrError(pl.blocker);
    if (!pl.job) continue;
    out.push(await ipc("job.start", { job: pl.job, wait }));
  }
  return out;
}
