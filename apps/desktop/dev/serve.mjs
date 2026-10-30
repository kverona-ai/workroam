// Development server for the desktop UI without Tauri: serves ../ui and
// bridges the page to a running workroamd (POST /ipc, GET /events as
// server-sent events). Usage: node dev/serve.mjs [port]
//
// The socket path follows workroam_core::paths::socket_path(); set
// WORKROAM_SOCKET to point it elsewhere.
import http from "node:http";
import net from "node:net";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const ui = path.join(here, "..", "ui");
const port = Number(process.argv[2] || 1420);

function socketPath() {
  if (process.env.WORKROAM_SOCKET) return process.env.WORKROAM_SOCKET;
  if (process.platform === "win32") return "\\\\.\\pipe\\workroamd-" + os.userInfo().username;
  if (process.env.WORKROAM_HOME) return path.join(process.env.WORKROAM_HOME, "workroamd.sock");
  if (process.platform === "darwin") return path.join(os.homedir(), "Library/Application Support/Workroam/workroamd.sock");
  if (process.env.XDG_RUNTIME_DIR) return path.join(process.env.XDG_RUNTIME_DIR, "workroam/workroamd.sock");
  return path.join(os.homedir(), ".workroam", "workroamd.sock");
}

export class Daemon {
  constructor() {
    this.next = 1;
    this.pending = new Map();
    this.listeners = new Set();
  }
  connect() {
    return new Promise((resolve, reject) => {
      const s = net.connect(socketPath());
      this.s = s;
      let buf = "";
      s.on("data", (d) => {
        buf += d;
        let i;
        while ((i = buf.indexOf("\n")) >= 0) {
          const line = buf.slice(0, i);
          buf = buf.slice(i + 1);
          if (!line.trim()) continue;
          const m = JSON.parse(line);
          if (m.id != null && this.pending.has(m.id)) {
            this.pending.get(m.id)(m);
            this.pending.delete(m.id);
          } else if (m.method) {
            for (const f of this.listeners) f(m);
          }
        }
      });
      s.on("error", reject);
      s.on("close", () => {
        for (const f of this.pending.values()) f({ error: { code: "daemon_unavailable", message: "connection closed" } });
        this.pending.clear();
        this.s = null;
      });
      s.on("connect", async () => {
        try {
          await this.call("hello", { protocol: 1, client: "desktop", version: "0.1.0" });
          await this.call("subscribe", {});
          resolve(this);
        } catch (e) {
          reject(e);
        }
      });
    });
  }
  call(method, params = {}) {
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, (m) => (m.error ? reject(m.error) : resolve(m.result)));
      this.s.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    });
  }
}

const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".png": "image/png", ".json": "application/json" };

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  let d = null;
  const daemon = async () => (d && d.s ? d : (d = await new Daemon().connect()));
  http.createServer(async (req, res) => {
    if (req.method === "POST" && req.url === "/ipc") {
      let body = "";
      for await (const c of req) body += c;
      const { method, params } = JSON.parse(body);
      try {
        const r = await (await daemon()).call(method, params || {});
        res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ ok: r ?? null }));
      } catch (e) {
        // Same shape as the Tauri command's error: workroam_core::Error.
        const x = e.data || {};
        const err = { code: x.name || "daemon_unavailable", message: e.message || String(e), hint: x.hint, data: x.data };
        res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ err }));
      }
      return;
    }
    if (req.url === "/events") {
      res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-store" });
      let dd;
      try { dd = await daemon(); } catch { res.end(); return; }
      const f = (m) => res.write(`data: ${JSON.stringify({ event: m.method, data: m.params })}\n\n`);
      dd.listeners.add(f);
      req.on("close", () => dd.listeners.delete(f));
      return;
    }
    const rel = decodeURIComponent(new URL(req.url, "http://x").pathname).replace(/^\/+/, "") || "index.html";
    const file = path.join(ui, path.normalize(rel));
    if (!file.startsWith(ui) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, { "content-type": types[path.extname(file)] || "application/octet-stream" });
    fs.createReadStream(file).pipe(res);
  }).listen(port, "127.0.0.1", () => console.log(`Workroam UI on http://127.0.0.1:${port}/  (daemon: ${socketPath()})`));
}
