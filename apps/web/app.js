// Workroam web reader UI: unlock, projects, HANDOFF.md, history and files.
import { Core, S3, Workspace, text } from "./reader.js";

const $ = (s, el = document) => el.querySelector(s);
const h = (tag, attrs = {}, ...kids) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (v !== false && v != null) e.setAttribute(k, v === true ? "" : v);
  }
  for (const k of kids.flat()) if (k != null) e.append(k.nodeType ? k : String(k));
  return e;
};
const store = {
  get(k) { try { return JSON.parse(localStorage.getItem(k)); } catch { return null; } },
  set(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch {} },
  del(k) { try { localStorage.removeItem(k); } catch {} },
};

const size = (n) => {
  if (n == null) return "";
  const u = ["B", "KB", "MB", "GB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${i ? n.toFixed(1) : n} ${u[i]}`;
};
const when = (t) => {
  const d = new Date(t);
  return isNaN(d) ? "" : d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
};
const lang = (navigator.language || "en").startsWith("zh") ? "zh" : "en";
const T = {
  zh: { unlock: "解锁", pairing: "配对码", storage: "存储位置", password: "加密密码或恢复密钥", code: "只读配对码（workroam device add --read-only）", remember: "在本浏览器记住存储位置（不保存密钥和密码）", projects: "项目", handoff: "交接文档", history: "同步历史", files: "文件", lock: "锁定", download: "下载", noHandoff: "这个快照里没有 HANDOFF.md", loading: "正在读取…", archived: "已归档", empty: "工作区里还没有项目", privacy: "数据在你的浏览器里解密，服务器看不到明文。", cors: "浏览器无法访问存储：请为桶配置 CORS（见 deploy/README.md）", back: "返回" },
  en: { unlock: "Unlock", pairing: "Pairing code", storage: "Storage", password: "Encryption password or recovery key", code: "Read-only pairing code (workroam device add --read-only)", remember: "Remember the storage location in this browser (never keys or passwords)", projects: "Projects", handoff: "Handoff", history: "History", files: "Files", lock: "Lock", download: "Download", noHandoff: "This snapshot has no HANDOFF.md", loading: "Reading…", archived: "Archived", empty: "The workspace has no projects yet", privacy: "Data is decrypted in your browser; the server never sees plaintext.", cors: "The browser cannot reach the storage: configure CORS on the bucket (see deploy/README.md)", back: "Back" },
}[lang];

// Safe Markdown subset for HANDOFF.md: headings, lists, code, emphasis, links.
function markdown(src) {
  const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
  const inline = (s) => esc(s)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2" rel="noreferrer noopener" target="_blank">$1</a>');
  const out = [];
  let list = false, code = false;
  for (const line of src.replace(/<!--[\s\S]*?-->/g, "").split("\n")) {
    if (line.startsWith("```")) {
      if (list) { out.push("</ul>"); list = false; }
      out.push(code ? "</code></pre>" : "<pre><code>");
      code = !code;
      continue;
    }
    if (code) { out.push(esc(line) + "\n"); continue; }
    const m = line.match(/^(#{1,4})\s+(.*)/);
    const li = line.match(/^\s*[-*]\s+(.*)/);
    if (!li && list) { out.push("</ul>"); list = false; }
    if (m) out.push(`<h${m[1].length + 1}>${inline(m[2])}</h${m[1].length + 1}>`);
    else if (li) { if (!list) { out.push("<ul>"); list = true; } out.push(`<li>${inline(li[1])}</li>`); }
    else if (line.trim()) out.push(`<p>${inline(line)}</p>`);
  }
  if (list) out.push("</ul>");
  if (code) out.push("</code></pre>");
  return out.join("");
}

let core;
let ws;
const main = $("#main");

function fail(el, e) {
  // fetch() rejects with a bare TypeError when CORS (or the network) blocks it.
  const blocked = e instanceof TypeError && /fetch|network|load failed/i.test(e.message);
  const msg = blocked ? T.cors : e.message;
  el.replaceChildren(h("p", { class: "err" }, msg));
}

function connectView() {
  const saved = store.get("workroam.location") || {};
  const err = h("div");
  const pw = h("input", { type: "password", autocomplete: "current-password", placeholder: T.password, required: true });
  const codeBox = h("textarea", { rows: 4, placeholder: "wr1:…", spellcheck: "false" });
  const f = (name, label, value = "", attrs = {}) =>
    h("label", {}, h("span", {}, label), h("input", { name, value, spellcheck: "false", ...attrs }));
  const storage = h("div", { class: "grid", hidden: true },
    f("endpoint", "Endpoint", saved.endpoint || "", { placeholder: "https://s3.example.com" }),
    f("bucket", "Bucket", saved.bucket || ""),
    f("prefix", "Prefix", saved.prefix || ""),
    f("region", "Region", saved.region || "", { placeholder: "us-east-1" }),
    h("label", {}, h("span", {}, "Addressing"),
      h("select", { name: "addressing" }, h("option", { value: "path" }, "path"), h("option", { value: "virtual-host", selected: saved.addressing === "virtual-host" }, "virtual-host"))),
    f("access_key", "Access key (read-only)", "", { autocomplete: "off" }),
    f("secret_key", "Secret key", "", { type: "password", autocomplete: "off" }),
  );
  const pairing = h("div", {}, h("label", {}, h("span", {}, T.code), codeBox));
  let mode = saved.endpoint ? "storage" : "pairing";
  const tabs = h("div", { class: "seg", role: "tablist" },
    ["pairing", "storage"].map((m) => h("button", { type: "button", role: "tab", "aria-selected": String(m === mode), onclick: () => setMode(m) }, T[m])));
  const setMode = (m) => {
    mode = m;
    pairing.hidden = m !== "pairing";
    storage.hidden = m !== "storage";
    for (const b of tabs.children) b.setAttribute("aria-selected", String(b.textContent === T[m]));
  };
  const remember = h("input", { type: "checkbox", checked: !!saved.endpoint });
  const go = h("button", { class: "primary", type: "submit" }, T.unlock);
  const form = h("form", { class: "card connect", onsubmit: async (ev) => {
    ev.preventDefault();
    go.disabled = true;
    err.replaceChildren(h("p", { class: "dim" }, T.loading));
    try {
      core ||= await Core.load();
      let loc, creds;
      if (mode === "pairing") {
        const p = core.openPairing(codeBox.value, pw.value);
        loc = { endpoint: p.endpoint, bucket: p.bucket, prefix: p.prefix, region: p.region, addressing: p.addressing };
        creds = { access_key: p.access_key, secret_key: p.secret_key };
      } else {
        const v = (n) => $(`[name=${n}]`, storage).value.trim();
        loc = { endpoint: v("endpoint"), bucket: v("bucket"), prefix: v("prefix"), region: v("region"), addressing: v("addressing") };
        creds = { access_key: v("access_key"), secret_key: $("[name=secret_key]", storage).value };
      }
      if (remember.checked) store.set("workroam.location", loc);
      else store.del("workroam.location");
      ws = new Workspace(core, new S3(loc, creds));
      await ws.unlock(pw.value, (stage) => err.replaceChildren(h("p", { class: "dim" }, `${T.loading} ${stage}`)));
      pw.value = "";
      codeBox.value = "";
      projectsView();
    } catch (e) {
      fail(err, e);
    } finally {
      go.disabled = false;
    }
  } },
    h("h1", {}, "Workroam"),
    h("p", { class: "dim" }, T.privacy),
    tabs, pairing, storage,
    h("label", {}, h("span", {}, T.password), pw),
    h("label", { class: "check" }, remember, h("span", {}, T.remember)),
    go, err,
  );
  setMode(mode);
  main.replaceChildren(form);
  $("#lock").hidden = true;
}

function projectsView(selected) {
  $("#lock").hidden = false;
  const side = h("nav", { class: "side card" }, h("h2", { class: "cap" }, T.projects));
  const body = h("section", { class: "content" });
  if (!ws.projects.length) side.append(h("p", { class: "dim" }, T.empty));
  for (const p of ws.projects) {
    const item = h("button", { class: "item", "aria-current": String(p.pid === selected), onclick: () => {
      for (const b of side.querySelectorAll(".item")) b.setAttribute("aria-current", "false");
      item.setAttribute("aria-current", "true");
      projectView(body, p);
    } },
      h("span", { class: "mono" }, p.project),
      h("small", {}, p.state === "archived" ? T.archived : `${p.device_name || ""} · ${when(p.created_at)}`));
    side.append(item);
  }
  main.replaceChildren(h("div", { class: "layout" }, side, body));
  const first = ws.projects.find((p) => p.pid === selected) || ws.projects[0];
  if (first) side.querySelectorAll(".item")[ws.projects.indexOf(first)].click();
}

async function projectView(el, p) {
  const hist = ws.history(p.pid);
  const handoff = h("div", { class: "card md" }, h("p", { class: "dim" }, T.loading));
  const list = h("div", { class: "card list" });
  for (const s of hist.slice(0, 200)) {
    list.append(h("button", { class: "row", onclick: () => filesView(el, p, s, []) },
      h("span", { class: "mono" }, s.id.slice(0, 8)),
      h("span", {}, s.description || s.label || ""),
      h("span", { class: "dim" }, s.hostname || ""),
      h("span", { class: "dim num" }, when(s.time))));
  }
  el.replaceChildren(
    h("header", {}, h("h1", { class: "mono" }, p.project), h("p", { class: "dim" }, `${hist.length} snapshots`)),
    h("h2", { class: "cap" }, T.handoff), handoff,
    h("h2", { class: "cap" }, T.history), list,
  );
  const latest = hist.find((s) => s.id.startsWith(p.snapshot || "")) || hist[0];
  if (!latest) { handoff.replaceChildren(h("p", { class: "dim" }, T.noHandoff)); return; }
  try {
    const n = await ws.find(latest.tree, "HANDOFF.md");
    if (!n) { handoff.replaceChildren(h("p", { class: "dim" }, T.noHandoff)); return; }
    handoff.innerHTML = markdown(text(await ws.read(n)));
  } catch (e) {
    fail(handoff, e);
  }
}

async function filesView(el, p, s, path) {
  const box = h("div", { class: "card list" }, h("p", { class: "dim" }, T.loading));
  const crumbs = h("nav", { class: "crumbs" },
    h("button", { onclick: () => projectView(el, p) }, `← ${p.project}`),
    h("button", { onclick: () => filesView(el, p, s, []) }, s.id.slice(0, 8)),
    path.map((seg, i) => h("button", { onclick: () => filesView(el, p, s, path.slice(0, i + 1)) }, seg.name)));
  const preview = h("div");
  el.replaceChildren(h("header", {}, h("h1", { class: "mono" }, p.project), h("p", { class: "dim" }, `${when(s.time)} · ${s.hostname || ""}`)), crumbs, box, preview);
  try {
    const tree = await ws.tree(path.length ? path[path.length - 1].subtree : s.tree);
    const nodes = [...(tree.nodes || [])].sort((a, b) => (a.type === "dir") === (b.type === "dir") ? a.name.localeCompare(b.name) : a.type === "dir" ? -1 : 1);
    box.replaceChildren(...nodes.map((n) => h("button", { class: "row", onclick: () => n.type === "dir" ? filesView(el, p, s, [...path, n]) : n.type === "file" && openFile(preview, n) },
      h("span", { class: "mono" }, (n.type === "dir" ? "▸ " : "") + n.name),
      h("span", {}, n.type === "symlink" ? `→ ${n.linktarget || ""}` : ""),
      h("span", { class: "dim num" }, n.type === "file" ? size(n.size) : ""),
      h("span", { class: "dim num" }, when(n.mtime)))));
  } catch (e) {
    fail(box, e);
  }
}

async function openFile(el, n) {
  el.replaceChildren(h("div", { class: "card" }, h("p", { class: "dim" }, T.loading)));
  try {
    const data = await ws.read(n);
    const url = URL.createObjectURL(new Blob([data]));
    const dl = h("a", { class: "btn", href: url, download: n.name }, T.download);
    const asText = data.length < 1 << 20 && !data.slice(0, 4096).includes(0);
    const view = !asText ? h("p", { class: "dim" }, size(data.length))
      : n.name.toLowerCase().endsWith(".md") ? Object.assign(h("div", { class: "md" }), { innerHTML: markdown(text(data)) })
      : h("pre", {}, text(data));
    el.replaceChildren(h("div", { class: "card preview" }, h("header", {}, h("strong", { class: "mono" }, n.name), dl), view));
    el.scrollIntoView({ behavior: "smooth", block: "start" });
  } catch (e) {
    fail(el, e);
  }
}

$("#lock").textContent = T.lock;
$("#lock").addEventListener("click", () => {
  core?.lock();
  ws = null;
  connectView();
});
connectView();
