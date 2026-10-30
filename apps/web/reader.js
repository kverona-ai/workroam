// Workroam web reader: S3 access (SigV4 with a read-only key, signed in the
// browser) and the WebAssembly core that unlocks and decrypts. Nothing
// decrypted is sent anywhere; the server only serves these files.

const te = new TextEncoder();
const td = new TextDecoder();

// ---------------------------------------------------------------- wasm --

export class Core {
  static async load(url = "workroam_web.wasm") {
    const r = await fetch(url);
    if (!r.ok) throw new Error(`cannot load ${url}`);
    const { instance } = await WebAssembly.instantiate(await r.arrayBuffer(), {});
    return new Core(instance.exports);
  }
  constructor(x) {
    this.x = x;
  }
  #put(bytes) {
    const p = this.x.wr_alloc(bytes.length);
    new Uint8Array(this.x.memory.buffer, p, bytes.length).set(bytes);
    return [p, bytes.length];
  }
  #call(name, ...args) {
    const ptrs = [];
    const flat = [];
    for (const a of args) {
      if (typeof a === "number") {
        flat.push(a);
        continue;
      }
      const b = typeof a === "string" ? te.encode(a) : a;
      const [p, n] = this.#put(b);
      ptrs.push([p, n]);
      flat.push(p, n);
    }
    const rc = this.x[name](...flat);
    const out = new Uint8Array(this.x.memory.buffer, this.x.wr_out_ptr(), this.x.wr_out_len()).slice();
    for (const [p, n] of ptrs) this.x.wr_free(p, n);
    if (rc === 0) return out;
    const e = new Error(td.decode(out));
    e.wrongPassword = rc === 1;
    throw e;
  }
  unlock(keyfile, secret) { this.#call("wr_unlock", keyfile, secret); }
  lock() { this.x.wr_lock(); }
  file(bytes) { return this.#call("wr_file", bytes); }
  json(bytes) { return JSON.parse(td.decode(this.file(bytes))); }
  index(bytes) { return Number(td.decode(this.#call("wr_index", bytes))); }
  locate(id) { return JSON.parse(td.decode(this.#call("wr_locate", id))); }
  blob(id, bytes) { return this.#call("wr_blob", id, bytes); }
  control(key, bytes) { return JSON.parse(td.decode(this.#call("wr_control", key, bytes))); }
  projectId(name) { return td.decode(this.#call("wr_project_id", name)); }
  openPairing(code, password) { return JSON.parse(td.decode(this.#call("wr_open_pairing", code, password))); }
}

// ------------------------------------------------------------------ S3 --

const hex = (b) => [...new Uint8Array(b)].map((x) => x.toString(16).padStart(2, "0")).join("");
async function sha256Hex(s) { return hex(await crypto.subtle.digest("SHA-256", te.encode(s))); }
async function hmac(key, msg) {
  const k = await crypto.subtle.importKey("raw", key, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return new Uint8Array(await crypto.subtle.sign("HMAC", k, te.encode(msg)));
}
function enc(s, slash = true) {
  return encodeURIComponent(s)
    .replace(/[!'()*]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase())
    .replace(/%2F/g, slash ? "%2F" : "/");
}

export class S3 {
  // loc: {endpoint, bucket, prefix, region, addressing}; creds: {access_key, secret_key}
  constructor(loc, creds) {
    this.loc = loc;
    this.creds = creds;
    this.region = loc.region || "us-east-1";
    const u = new URL(loc.endpoint);
    this.virtual = loc.addressing === "virtual-host";
    this.base = this.virtual ? `${u.protocol}//${loc.bucket}.${u.host}` : `${u.protocol}//${u.host}`;
    this.path0 = (u.pathname.replace(/\/$/, "") + (this.virtual ? "" : `/${loc.bucket}`));
    const p = (loc.prefix || "").replace(/^\/+|\/+$/g, "");
    this.root = p ? p + "/" : "";
  }
  async #signed(key, query = {}, extra = {}) {
    const path = this.path0 + "/" + key.split("/").map((s) => enc(s)).join("/");
    const url = new URL(this.base + (key === "" ? this.path0 + "/" : path));
    const qs = Object.keys(query).sort().map((k) => `${enc(k)}=${enc(query[k])}`).join("&");
    const now = new Date();
    const amzDate = now.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}/, "");
    const day = amzDate.slice(0, 8);
    const headers = { "x-amz-content-sha256": "UNSIGNED-PAYLOAD", "x-amz-date": amzDate };
    const canonical = ["GET", url.pathname, qs, `host:${url.host}\n` + `x-amz-content-sha256:UNSIGNED-PAYLOAD\nx-amz-date:${amzDate}\n`, "host;x-amz-content-sha256;x-amz-date", "UNSIGNED-PAYLOAD"].join("\n");
    const scope = `${day}/${this.region}/s3/aws4_request`;
    const toSign = ["AWS4-HMAC-SHA256", amzDate, scope, await sha256Hex(canonical)].join("\n");
    let k = await hmac(te.encode("AWS4" + this.creds.secret_key), day);
    for (const part of [this.region, "s3", "aws4_request"]) k = await hmac(k, part);
    const sig = hex(await hmac(k, toSign));
    headers.authorization = `AWS4-HMAC-SHA256 Credential=${this.creds.access_key}/${scope}, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=${sig}`;
    const r = await fetch(url.toString() + (qs ? "?" + qs : ""), { headers: { ...headers, ...extra }, cache: "no-store" });
    if (r.status === 404) return null;
    if (!r.ok) throw new Error(`storage: HTTP ${r.status} for ${key || "list"}`);
    return r;
  }
  async get(key, range) {
    const r = await this.#signed(this.root + key, {}, range ? { range: `bytes=${range[0]}-${range[1] - 1}` } : {});
    return r ? new Uint8Array(await r.arrayBuffer()) : null;
  }
  async list(prefix) {
    const out = [];
    let token = null;
    do {
      const q = { "list-type": "2", prefix: this.root + prefix };
      if (token) q["continuation-token"] = token;
      const r = await this.#signed("", q);
      if (!r) break;
      const doc = new DOMParser().parseFromString(await r.text(), "application/xml");
      for (const c of doc.getElementsByTagName("Contents")) {
        const k = c.getElementsByTagName("Key")[0].textContent;
        // Some stores list folder markers ("repo/keys/"); they are not objects.
        if (!k.endsWith("/")) out.push(k.slice(this.root.length));
      }
      const t = doc.getElementsByTagName("NextContinuationToken")[0];
      token = t ? t.textContent : null;
    } while (token);
    return out;
  }
}

async function pool(items, n, f) {
  const out = new Array(items.length);
  let i = 0;
  await Promise.all(Array.from({ length: Math.min(n, items.length) }, async () => {
    while (i < items.length) {
      const j = i++;
      out[j] = await f(items[j], j);
    }
  }));
  return out;
}

// ----------------------------------------------------------- workspace --

export class Workspace {
  constructor(core, s3) {
    this.core = core;
    this.s3 = s3;
    this.trees = new Map();
  }

  async unlock(secret, progress = () => {}) {
    progress("keys");
    const keys = await this.s3.list("repo/keys/");
    if (!keys.length) throw new Error("no Workroam workspace at this location");
    let ok = false;
    let last;
    for (const k of keys) {
      try {
        this.core.unlock(await this.s3.get(k), secret);
        ok = true;
        break;
      } catch (e) {
        last = e;
      }
    }
    if (!ok) throw last;
    progress("index");
    const idx = await this.s3.list("repo/index/");
    await pool(idx, 6, async (k) => this.core.index(await this.s3.get(k)));
    progress("snapshots");
    const snaps = await this.s3.list("repo/snapshots/");
    this.snapshots = (await pool(snaps, 6, async (k) => ({ id: k.split("/").pop(), ...this.core.json(await this.s3.get(k)) })))
      .sort((a, b) => (a.time < b.time ? 1 : -1));
    progress("projects");
    const heads = (await this.s3.list("control/projects/")).filter((k) => k.endsWith("/head"));
    this.projects = (await pool(heads, 6, async (k) => {
      try {
        const key = k.slice("control/".length);
        const h = this.core.control(key, await this.s3.get(k));
        return { pid: key.split("/")[1], ...h };
      } catch {
        return null;
      }
    })).filter(Boolean).sort((a, b) => a.project.localeCompare(b.project));
  }

  history(pid) {
    const tag = `workroam:project=${pid}`;
    return this.snapshots.filter((s) => (s.tags || []).includes(tag));
  }

  async blob(id) {
    const loc = this.core.locate(id);
    const data = await this.s3.get(`repo/data/${loc.pack.slice(0, 2)}/${loc.pack}`, [loc.offset, loc.offset + loc.length]);
    if (!data) throw new Error(`missing pack ${loc.pack}`);
    return this.core.blob(id, data);
  }

  async tree(id) {
    if (!this.trees.has(id)) this.trees.set(id, JSON.parse(td.decode(await this.blob(id))));
    return this.trees.get(id);
  }

  async read(node) {
    const parts = await pool(node.content || [], 4, (b) => this.blob(b));
    const n = parts.reduce((a, p) => a + p.length, 0);
    const out = new Uint8Array(n);
    let o = 0;
    for (const p of parts) {
      out.set(p, o);
      o += p.length;
    }
    return out;
  }

  // Find a file by path in a snapshot tree.
  async find(treeId, path) {
    let t = await this.tree(treeId);
    const segs = path.split("/");
    for (let i = 0; i < segs.length; i++) {
      const n = (t.nodes || []).find((x) => x.name === segs[i]);
      if (!n) return null;
      if (i === segs.length - 1) return n;
      if (n.type !== "dir") return null;
      t = await this.tree(n.subtree);
    }
    return null;
  }
}

export const text = (b) => td.decode(b);
