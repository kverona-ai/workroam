// The setup wizard (PRD 7.1, ONB-01 – ONB-09). Each screen maps to a
// `setup.step` the CLI's `workroam setup` sends in the same order.
import { t } from "./i18n.js";
import {
  ipc, act, h, icon, btn, field, input, callout, pill, seg, sw, copy, toast, showError, pickDir,
  saveFile, native, inTauri, onDaemon, $, sheet, acts, fill } from "./core.js";

const PRESETS = [
  ["rustfs", "server", "RF"],
  ["aws", "cloud", "S3"],
  ["r2", "cloud", "R2"],
  ["oss", "cloud", "OSS"],
  ["other", "bucket", "S3"],
  ["local", "folder", "/"],
];

export async function startSetup(root, done) {
  const st = { mode: null, begin: null, session: null, ep: null, buckets: [], info: null, pw: null };
  const rail = h("ol");
  const railMobile = h("span");
  const stage = h("div", { class: "stage" });
  const foot = h("div", { class: "win-foot" });
  fill(root, 
    h("div", { class: "win app-win" },
      h("div", { class: "win-body" },
        h("aside", { class: "rail" },
          h("div", { class: "rail-brand" }, h("svg", { class: "logo-sm", "aria-hidden": "true" }, h("use", { href: "#logo" })), "Workroam"),
          rail, h("div", { class: "rail-mobile" }, railMobile)),
        h("div", { class: "wiz-main" }, stage, foot))));

  const steps = {
    new: ["path", "storage", "credentials", "machine", "bucket", "checks", "password", "recovery", "done"],
    join: ["path", "pairing", "machine", "unlock", "projects", "done"],
    from_backup: ["path", "backup", "machine", "unlock", "as_device", "projects", "done"],
  };
  const order = () => steps[st.mode || "new"];
  const step = (name, p = {}) => ipc("setup.step", { session: st.session, step: name, ...p });

  function drawRail(cur) {
    const list = order();
    const i = list.indexOf(cur);
    fill(rail, ...list.map((s, j) => h("li", { class: j < i ? "done" : j === i ? "cur" : "" },
      h("span", { class: "n" }, j < i ? icon("check") : String(j + 1)), t(`step.${s}`))));
    railMobile.textContent = `${i + 1} / ${list.length} · ${t(`step.${cur}`)}`;
  }

  // One screen: body nodes, then the footer (hint + back + next).
  function screen(name, title, sub, body, { next, nextLabel = t("continue"), back = true, hint } = {}) {
    drawRail(name);
    const err = h("div");
    const nextBtn = next ? btn(nextLabel, async () => {
      nextBtn.disabled = true;
      fill(err, );
      try {
        await next();
      } catch (e) {
        fill(err, callout("bad", "warn", e.message, e.hint ? h("div", { class: "hint" }, e.hint) : null));
      } finally {
        nextBtn.disabled = false;
      }
    }, { cls: "primary big" }) : null;
    fill(stage, h("section", { class: "step" },
      h("div", {}, h("h2", {}, title), sub ? h("p", { class: "sub" }, sub) : null), ...body, err));
    fill(foot, h("span", { class: "hint" }, hint || ""),
      back ? btn(t("back"), () => go(prev(name)), { cls: "ghost" }) : null, nextBtn);
    stage.scrollTop = 0;
    setTimeout(() => stage.querySelector("input:not([type=checkbox]):not([type=radio])")?.focus(), 30);
    return { nextBtn, err };
  }
  const prev = (name) => order()[Math.max(0, order().indexOf(name) - 1)];
  const go = (name) => screens[name]();

  const screens = {
    // Welcome: new / join / restore from an off-site backup.
    path: act("setup.path", async () => {
      if (st.session) await ipc("setup.cancel", { session: st.session }).catch(() => {});
      st.session = null;
      const opt = (v, glyph, cls, title, d) => h("label", { class: "choice" },
        h("input", { type: "radio", name: "path", value: v, checked: (st.mode || "new") === v }),
        h("span", { class: `glyph ${cls}` }, icon(glyph)), h("span", {}, h("span", { class: "t" }, title), h("span", { class: "d" }, d)));
      const box = h("div", { class: "grid1" },
        opt("new", "sparkle", "", t("path.new"), t("path.new.d")),
        opt("join", "link", "mint", t("path.join"), t("path.join.d")),
        opt("from_backup", "restore", "amber", t("path.backup"), t("path.backup.d")));
      screen("path", t("welcome.title"), t("welcome.sub"), [relayDemo(), box], {
        back: false,
        next: async () => {
          st.mode = $("input[name=path]:checked", box).value;
          st.begin = await ipc("setup.begin", { mode: st.mode });
          st.session = st.begin.session;
          go(order()[1]);
        },
      });
    }),

    // Storage preset and endpoint; addressing and region are detected live.
    storage: act("setup.storage", () => {
      let preset = st.preset || "rustfs";
      const ep = input({ mono: true, placeholder: "https://s3.example.com", value: st.endpoint || "" });
      const region = input({ mono: true, placeholder: "auto", value: st.region || "" });
      const addr = h("select", { class: "input" }, h("option", { value: "" }, t("auto_detect")), h("option", { value: "path" }, "path"), h("option", { value: "virtual-host" }, "virtual-host"));
      const chips = h("div", { class: "chips" });
      const hint = h("div");
      const grid = h("div", { class: "grid3" }, PRESETS.map(([id, glyph, mono]) => h("label", { class: "choice col" },
        h("input", { type: "radio", name: "preset", value: id, checked: id === preset, onchange: () => { preset = id; refresh(); } }),
        h("span", { class: "mono-glyph" }, mono), h("span", { class: "t" }, t(`preset.${id}`)), h("span", { class: "d" }, t(`preset.${id}.d`)))));
      const epField = field(t("endpoint"), ep);
      const adv = h("div", { class: "grid2" }, field(t("region"), region), field(t("addressing"), addr));
      let timer;
      // setup.addressing: shown under the endpoint as soon as it is typed.
      const detect = act("setup.addressing", async () => {
        fill(chips, );
        if (!ep.value.trim() && !["aws", "r2", "oss"].includes(preset)) return;
        try {
          st.ep = await step("endpoint", { preset, endpoint: ep.value.trim(), addressing: addr.value || null, region: region.value.trim() || null });
          fill(chips, pill(`${t("addressing")}: ${st.ep.addressing}`, "acc"), pill(`${t("region")}: ${st.ep.region || "—"}`, "acc"),
            st.ep.can_create_key ? pill(t("can_create_key"), "ok", "key") : null);
        } catch (e) {
          fill(chips, pill(e.message, "warn", "warn"));
        }
      });
      function refresh() {
        ep.placeholder = preset === "local" ? t("local_dir") : preset === "r2" ? "https://<account>.r2.cloudflarestorage.com" : preset === "oss" ? "https://oss-cn-hangzhou.aliyuncs.com" : "https://s3.example.com";
        epField.querySelector(".lbl").textContent = preset === "local" ? t("local_dir") : t("endpoint");
        fill(hint, preset === "rustfs" ? callout("info", "server", h("div", {}, t("rustfs_hint")),
          h("div", { class: "codebox" }, "curl -fsSL https://raw.githubusercontent.com/kverona-ai/workroam/main/deploy/install.sh | sh -s -- --domain s3.example.com"),
          btn(t("copy"), () => copy("curl -fsSL https://raw.githubusercontent.com/kverona-ai/workroam/main/deploy/install.sh | sh -s -- --domain s3.example.com"), { cls: "sm", ic: "copy" })) : null);
        adv.hidden = preset === "local";
        detect();
      }
      ep.addEventListener("input", () => { clearTimeout(timer); timer = setTimeout(detect, 500); });
      region.addEventListener("change", detect);
      addr.addEventListener("change", detect);
      screen("storage", t("storage.title"), t("storage.sub"), [grid, epField, chips, adv, hint], {
        next: async () => {
          st.preset = preset;
          st.endpoint = ep.value.trim();
          st.region = region.value.trim();
          st.ep = await step("endpoint", { preset, endpoint: st.endpoint, addressing: addr.value || null, region: st.region || null });
          if (preset === "local") {
            st.buckets = await step("credentials", {});
            await step("key_mode", { mode: "current" });
            return go("machine");
          }
          go("credentials");
        },
      });
      refresh();
    }),

    // Access key, then how this device's key is scoped.
    credentials: act("setup.credentials", () => {
      const ak = input({ mono: true, value: st.ak || "", placeholder: "AKIA…" });
      const sk = input({ mono: true, type: "password", placeholder: "••••••••" });
      const eye = btn("", () => (sk.type = sk.type === "password" ? "text" : "password"), { cls: "icon", ic: "eye", title: t("show") });
      const can = st.ep?.can_create_key;
      let mode = can ? "create" : "current";
      let access = "read-write";
      const token = input({ mono: true, type: "password", placeholder: t("admin_token") });
      const tokenField = field(t("admin_token"), token);
      tokenField.hidden = !(can && st.ep?.needs_admin_token);
      const modeBox = h("div", { class: "grid1" });
      // setup.key_mode: create a scoped key (read-write / read-only) or keep this one.
      const keyMode = act("setup.key_mode", () => {
        const o = (v, title, d) => h("label", { class: "choice" }, h("input", { type: "radio", name: "km", value: v, checked: v === mode, onchange: () => { mode = v; tokenField.hidden = !(v.startsWith("create") && st.ep?.needs_admin_token); } }),
          h("span", { class: "glyph" }, icon(v === "current" ? "key" : "shield")), h("span", {}, h("span", { class: "t" }, title), h("span", { class: "d" }, d)));
        fill(modeBox, ...(can ? [o("create", t("km.create"), t("km.create.d")), o("create-ro", t("km.create_ro"), t("km.create_ro.d"))] : []), o("current", t("km.current"), can ? t("km.current.d") : t("km.current.only")));
      });
      keyMode();
      screen("credentials", t("credentials.title"), t("credentials.sub"), [
        h("div", { class: "grid2" }, field("Access Key ID", ak), field("Secret Access Key", h("div", { class: "inrow" }, sk, eye))),
        h("h4", { class: "cap" }, t("key_mode")), modeBox, tokenField,
        callout("info", "lock", t("credentials.keychain")),
      ], {
        next: async () => {
          st.ak = ak.value.trim();
          st.buckets = await step("credentials", { access_key: st.ak, secret_key: sk.value });
          if (mode === "create-ro") { access = "read-only"; }
          await step("key_mode", { mode: mode.startsWith("create") ? "create" : "current", access, admin_token: token.value || null });
          go("machine");
        },
      });
    }),

    machine: act("setup.machine", () => {
      const d = st.begin.defaults;
      const name = input({ value: st.machine?.device_name || d.device_name });
      const root = input({ mono: true, value: st.machine?.root || d.root });
      const pick = btn(t("choose"), async () => { const p = await pickDir(t("root"), root.value); if (p) root.value = p; }, { cls: "sm", ic: "folder" });
      const reach = input({ mono: true, placeholder: "git.corp.example, 10.0.0.0/8", value: (st.machine?.reachable || []).join(", ") });
      let auto = st.machine?.auto || "ask";
      const autoSeg = seg([["ask", t("auto.ask")], ["auto", t("auto.auto")], ["off", t("auto.off")]], auto, (v) => (auto = v));
      // settings.autostart is also offered here, as in the CLI wizard.
      let autostart = st.machine?.autostart ?? true;
      const as = sw(autostart, (v) => (autostart = v), t("autostart"));
      const agents = (st.begin.agents || []).filter((a) => a.installed);
      screen("machine", t("machine.title"), t("machine.sub"), [
        h("div", { class: "grid2" }, field(t("device_name"), name), field(t("root"), h("div", { class: "inrow" }, root, pick))),
        field(t("reachable"), reach, t("reachable.hint")),
        h("div", { class: "setgrp" },
          h("div", { class: "setrow" }, h("div", { class: "tx" }, h("b", {}, t("auto_sync")), h("span", {}, t("auto_sync.d"))), autoSeg),
          h("div", { class: "setrow" }, h("div", { class: "tx" }, h("b", {}, t("autostart")), h("span", {}, t("autostart.d"))), as)),
        agents.length ? h("div", { class: "chips" }, h("span", { class: "hint" }, t("agents_found")), agents.map((a) => pill(`${a.name} ${a.version || ""}`, a.supported === false ? "warn" : "ok", "check"))) : null,
      ], {
        next: async () => {
          st.machine = { device_name: name.value.trim(), root: root.value.trim(), reachable: reach.value.split(",").map((x) => x.trim()).filter(Boolean), auto, autostart };
          await step("machine", { machine: st.machine });
          if (inTauri) native("plugin:autostart|" + (autostart ? "enable" : "disable")).catch(() => {});
          go(order()[order().indexOf("machine") + 1]);
        },
      });
    }),

    // Bucket: a new one by default, an existing one, or a custom name.
    bucket: act("setup.bucket", () => {
      if (st.preset === "local") {
        return step("bucket", { choice: "custom", name: "workspace", prefix: "" }).then(() => go("checks"), showError);
      }
      let choice = "auto";
      const name = input({ mono: true, placeholder: `workroam-${(st.machine?.device_name || "").toLowerCase()}-xxxx` });
      const prefix = input({ mono: true, placeholder: t("prefix.ph") });
      const list = h("div", { class: "checks" }, (st.buckets || []).map((b) => h("label", { class: "chk pick" },
        h("input", { class: "cb", type: "radio", name: "bk", value: b.name, onchange: () => { choice = "existing"; name.value = b.name; } }),
        h("span", { class: "nm mono" }, b.name), b.workspace ? pill(t("bucket.is_ws"), "acc") : b.empty ? pill(t("bucket.empty"), "ok") : pill(t("bucket.nonempty"), "warn"))));
      const o = (v, title, d) => h("label", { class: "choice" }, h("input", { type: "radio", name: "bc", value: v, checked: v === choice, onchange: () => { choice = v; list.hidden = v !== "existing"; nameF.hidden = v !== "custom"; } }),
        h("span", { class: "glyph" }, icon(v === "auto" ? "sparkle" : v === "existing" ? "bucket" : "doc")), h("span", {}, h("span", { class: "t" }, title), h("span", { class: "d" }, d)));
      const nameF = field(t("bucket.name"), name, t("bucket.rules"));
      nameF.hidden = true;
      list.hidden = true;
      const prefixF = field(t("prefix"), prefix, t("prefix.hint"));
      // setup.prefix: required in a non-empty bucket that is not a workspace.
      act("setup.prefix", () => {})();
      // setup.workspace_found: the daemon refuses a bucket that already holds a workspace.
      const found = act("setup.workspace_found", (e) => callout("warn", "warn", e.message, " ", btn(t("switch_to_join"), () => { st.mode = "join"; screens.path(); }, { cls: "sm" })));
      const s = screen("bucket", t("bucket.title"), t("bucket.sub"), [
        h("div", { class: "grid1" }, o("auto", t("bucket.auto"), t("bucket.auto.d")), st.buckets?.length ? o("existing", t("bucket.existing"), t("bucket.existing.d")) : null, o("custom", t("bucket.custom"), t("bucket.custom.d"))),
        list, nameF, prefixF,
      ], {
        next: async () => {
          try {
            const info = await step("bucket", { choice, name: choice === "auto" ? null : name.value.trim(), prefix: prefix.value.trim() });
            st.bucket = info;
            go("checks");
          } catch (e) {
            if (e.hint && e.hint.includes("--join")) { fill(s.err, found(e)); return; }
            throw e;
          }
        },
      });
    }),

    // 16 checks, refreshed one by one (setup.check_result events).
    checks: act("setup.checks", async () => {
      const box = h("div", { class: "checks" });
      const sum = h("div", { class: "summary" });
      const extra = h("div");
      let unlisten;
      const s = screen("checks", t("checks.title"), t("checks.sub"), [sum, box, extra, h("div", { class: "acts-row" }, btn(t("recheck"), () => run(), { cls: "sm", ic: "refresh" }))], {
        next: async () => {
          if (!st.checksPassed) throw new Error(t("checks.blocked"));
          go("password");
        },
      });
      s.nextBtn.disabled = true;
      const rows = new Map();
      const draw = (r) => {
        const el = checkRow(r);
        if (rows.has(r.id)) rows.get(r.id).replaceWith(el); else box.append(el);
        rows.set(r.id, el);
      };
      async function run() {
        fill(box, ); rows.clear(); fill(extra, );
        fill(sum, h("span", { class: "spin" }), h("span", { class: "hint" }, t("checks.running")));
        unlisten ||= await onDaemon((ev) => ev.event === "setup.check_result" && ev.data.session === st.session && draw(ev.data.result));
        try {
          const r = await ipc("setup.check", { session: st.session });
          r.results.forEach(draw);
          st.checksPassed = r.passed;
          const n = (k) => r.results.filter((x) => x.status === k).length;
          fill(sum, pill(`${n("pass")} ${t("passed")}`, "ok", "check"), n("warn") ? pill(`${n("warn")} ${t("warnings")}`, "warn", "warn") : null, n("fail") ? pill(`${n("fail")} ${t("failed")}`, "bad", "x") : null);
          s.nextBtn.disabled = !r.passed;
          if (!r.passed && r.broad_key) fill(extra, broadKey(run));
          else if (!r.passed && r.blocker) fill(extra, callout("bad", "warn", r.blocker.message, r.blocker.hint ? h("div", { class: "hint" }, r.blocker.hint) : null));
        } catch (e) {
          fill(sum, callout("bad", "warn", e.message));
        }
      }
      // setup.broad_key: a key that sees more than this workspace needs an explicit yes.
      const broadKey = act("setup.broad_key", (rerun) => {
        const cb = h("input", { class: "cb", type: "checkbox" });
        return callout("warn", "warn", h("div", {}, h("b", {}, t("broad.title")), h("div", {}, t("broad.d"))),
          h("label", { class: "inline" }, cb, t("broad.ack")),
          btn(t("continue"), async () => { if (!cb.checked) return toast(t("broad.tick"), "warn"); await step("accept_broad_key"); rerun(); }, { cls: "sm" }));
      });
      run();
    }),

    password: act("setup.password", () => {
      const a = input({ type: "password", autocomplete: "new-password" });
      const b = input({ type: "password", autocomplete: "new-password" });
      const meter = h("div", { class: "meter" }, [0, 1, 2, 3, 4].map(() => h("i")));
      const txt = h("div", { class: "hint" }, t("pw.hint"));
      const score = (p) => Math.min(4, (p.length >= 10) + (p.length >= 14) + (/[A-Z]/.test(p) && /[a-z]/.test(p)) + /\d/.test(p) + /[^A-Za-z0-9]/.test(p) + (p.length >= 20) - (p.length < 8 ? 2 : 0));
      a.addEventListener("input", () => {
        const s = Math.max(0, score(a.value));
        const col = ["var(--bad)", "var(--bad)", "var(--warn)", "var(--ok)", "var(--ok)"][s];
        [...meter.children].forEach((i, j) => (i.style.background = j <= s && a.value ? col : ""));
        txt.textContent = a.value ? t(`pw.s${s}`) : t("pw.hint");
      });
      screen("password", t("password.title"), t("password.sub"), [
        field(t("password"), a), meter, txt, field(t("password_again"), b),
        callout("warn", "lock", t("password.warn")),
      ], {
        next: async () => {
          if (a.value !== b.value) throw new Error(t("pw.mismatch"));
          st.pw = await step("password", { password: a.value });
          go("recovery");
        },
      });
    }),

    // Recovery key: copy / download .txt / print, then type two groups back.
    recovery: act("setup.recovery_key", () => {
      const rk = st.pw.recovery_key;
      const groups = rk.split("-");
      const [ga, gb] = st.pw.confirm_groups;
      const grid = h("div", { class: "rkey" }, groups.map((g, i) => h("span", {}, g, h("small", {}, String(i + 1)))));
      const typed = input({ mono: true, placeholder: `${groups[ga - 1].replace(/./g, "•")} ${groups[gb - 1].replace(/./g, "•")}` });
      const download = async () => {
        const p = await saveFile(t("rk.save"), "workroam-recovery-key.txt", [{ name: "Text", extensions: ["txt"] }]);
        if (!p) return;
        await native("save_private_text", { path: p, text: `Workroam recovery key\n\n${rk}\n` });
        toast(t("saved"), "ok");
      };
      screen("recovery", t("recovery.title"), t("recovery.sub"), [
        grid,
        h("div", { class: "acts-row" }, btn(t("copy"), () => copy(rk), { cls: "sm", ic: "copy" }), btn(t("download_txt"), download, { cls: "sm", ic: "download" }), btn(t("print"), () => printKey(rk), { cls: "sm", ic: "print" })),
        field(t("rk.confirm", { a: ga, b: gb }), typed),
      ], {
        next: async () => {
          await step("confirm_recovery", { typed: typed.value });
          go("done");
        },
      });
    }),

    // Join: paste a pairing code, scan its QR code, or a 6-digit code + server.
    pairing: act("setup.pairing_code", () => {
      const code = h("textarea", { class: "input mono", rows: 4, placeholder: "wr1:…  /  123 456", spellcheck: "false" });
      const server = input({ mono: true, placeholder: "https://workroam.example.com" });
      const serverF = field(t("server_url"), server, t("server_url.hint"));
      serverF.hidden = true;
      code.addEventListener("input", () => {
        const v = code.value.trim();
        serverF.hidden = !(v.replace(/\D/g, "").length === 6 && !v.startsWith("wr1:"));
      });
      const video = h("video", { class: "scan", playsinline: true, muted: true, hidden: true });
      const scan = btn(t("scan_qr"), () => scanQr(video, (txt) => { code.value = txt; code.dispatchEvent(new Event("input")); }), { cls: "sm", ic: "eye" });
      const fromImage = btn(t("qr_image"), () => qrFromImage((txt) => { code.value = txt; code.dispatchEvent(new Event("input")); }), { cls: "sm", ic: "doc" });
      screen("pairing", t("pairing.title"), t("pairing.sub"), [field(t("pairing_code"), code), h("div", { class: "acts-row" }, scan, fromImage), video, serverF], {
        next: async () => {
          stopScan(video);
          await step("pairing_code", { code: code.value.trim(), server: serverF.hidden ? null : server.value.trim(), device_name: st.begin.defaults.device_name });
          go("machine");
        },
      });
    }),

    backup: () => {
      const ep = input({ mono: true, placeholder: "https://s3.backup.example  /  /Volumes/nas/workroam" });
      const bucket = input({ mono: true });
      const prefix = input({ mono: true });
      const ak = input({ mono: true });
      const sk = input({ mono: true, type: "password" });
      screen("backup", t("backup_src.title"), t("backup_src.sub"), [
        field(t("endpoint_or_dir"), ep), h("div", { class: "grid2" }, field(t("bucket"), bucket), field(t("prefix"), prefix)),
        h("div", { class: "grid2" }, field("Access Key ID", ak), field("Secret Access Key", sk)),
      ], {
        next: async () => {
          const local = !/^https?:\/\//.test(ep.value.trim());
          await step("backup_source", { endpoint: ep.value.trim(), bucket: bucket.value.trim(), prefix: prefix.value.trim(), preset: local ? "local" : "other", access_key: local ? null : ak.value.trim(), secret_key: local ? null : sk.value });
          go("machine");
        },
      });
    },

    // setup.unlock: the encryption password, or the recovery key.
    unlock: act("setup.unlock", () => {
      const pw = input({ type: "password", autocomplete: "current-password" });
      let useRk = false;
      const f = field(t("password"), pw);
      const forgot = btn(t("forgot_pw"), () => {
        useRk = !useRk;
        f.querySelector(".lbl").textContent = useRk ? t("recovery_key") : t("password");
        pw.type = useRk ? "text" : "password";
        pw.classList.toggle("mono", useRk);
        forgot.textContent = useRk ? t("use_pw") : t("forgot_pw");
      }, { cls: "ghost sm" });
      screen("unlock", t("unlock.title"), t("unlock.sub"), [f, forgot], {
        next: async () => {
          st.info = await step("unlock", useRk ? { recovery_key: pw.value } : { password: pw.value });
          go(order()[order().indexOf("unlock") + 1]);
        },
      });
    }),

    // setup.as_device: after restoring from a backup, take a former device's place.
    as_device: act("setup.as_device", () => {
      let dev = "";
      const o = (v, title) => h("label", { class: "choice" }, h("input", { type: "radio", name: "asd", value: v, checked: v === dev, onchange: () => (dev = v) }), h("span", { class: "glyph" }, icon(v ? "laptop" : "plus")), h("span", {}, h("span", { class: "t" }, title)));
      screen("as_device", t("as_device.title"), t("as_device.sub"), [h("div", { class: "grid1" }, o("", t("as_device.new")), (st.info?.devices || []).map((d) => o(d, t("as_device.as", { name: d }))))], {
        next: async () => { st.asDevice = dev || null; go("projects"); },
      });
    }),

    // setup.projects: tick projects, each with a folder on this computer.
    projects: act("setup.projects", () => {
      const all = st.info?.projects || [];
      const root = st.machine?.root || st.begin.defaults.root;
      const sep = root.includes("\\") ? "\\" : "/";
      const rows = all.map((p) => {
        const cb = h("input", { class: "cb", type: "checkbox", checked: true });
        const dir = input({ mono: true, value: `${root.replace(/[\\/]$/, "")}${sep}${p}` });
        const pick = btn("", async () => { const d = await pickDir(p, dir.value); if (d) dir.value = d; }, { cls: "icon sm", ic: "folder", title: t("choose") });
        return { p, cb, dir, el: h("div", { class: "repo" }, cb, h("span", { class: "nm" }, p), h("span", { class: "end grow" }, dir), pick) };
      });
      screen("projects", t("projects.title"), all.length ? t("projects.sub") : t("projects.none"), [all.length ? h("div", { class: "group" }, rows.map((r) => r.el)) : null], {
        next: async () => {
          const list = rows.filter((r) => r.cb.checked).map((r) => ({ name: r.p, dir: r.dir.value.trim() }));
          await step("projects", { projects: list, all: false, as_device: st.asDevice || null });
          go("done");
        },
        nextLabel: t("finish"),
      });
    }),

    // setup.done: commit, then show the pairing code and its QR code.
    done: act("setup.done", async () => {
      drawRail("done");
      fill(stage, h("section", { class: "step" }, h("div", { class: "acts-row" }, h("span", { class: "spin" }), t("committing"))));
      fill(foot, );
      let r;
      try {
        r = await ipc("setup.commit", { session: st.session });
      } catch (e) {
        fill(stage, h("section", { class: "step" }, callout("bad", "warn", e.message, e.hint ? h("div", { class: "hint" }, e.hint) : null)));
        fill(foot, btn(t("back"), () => go(prev("done")), { cls: "ghost" }));
        return;
      }
      st.session = null;
      const body = [h("div", { class: "acts-row" }, h("span", { class: "success-orb" }, icon("check")), h("div", {}, h("h2", {}, t("done.title")), h("p", { class: "sub" }, `${r.device_name} · ${r.workspace}`)))];
      if (r.created_bucket) body.push(callout("ok", "check", t("done.bucket")));
      if (r.created_key) body.push(callout("ok", "key", t("done.key")));
      if (r.pairing_code) {
        body.push(h("h4", { class: "cap" }, t("pairing_code")), h("div", { class: "pairbox" },
          r.pairing_qr_svg ? h("div", { class: "qr", html: r.pairing_qr_svg }) : null,
          h("div", { class: "grow" }, h("p", { class: "hint" }, t("done.pairing")), h("div", { class: "codebox wrap" }, r.pairing_code), btn(t("copy"), () => copy(r.pairing_code), { cls: "sm", ic: "copy" }))));
      }
      if (r.projects?.length) body.push(callout("info", "down", t("done.pull", { n: r.projects.length })));
      fill(stage, h("section", { class: "step" }, ...body));
      fill(foot, h("span", { class: "hint" }, ""), btn(st.mode === "new" ? t("first_project") : t("open_app"), () => done({ firstProject: st.mode === "new", pull: r.projects }), { cls: "primary big" }));
    }),
  };

  // home.first_run: the wizard opens by itself until the computer is set up.
  act("home.first_run", () => screens.path())();
}

function relayDemo() {
  const node = (cls, ic, b, s) => h("div", { class: `rnode ${cls}` }, h("span", { class: "rd" }, icon(ic, "lg")), h("b", {}, b), h("small", {}, s));
  return h("div", { class: "relay demo" }, h("span", { class: "rorb" }),
    h("div", { class: "track" }, node("co", "building", t("relay.office"), t("relay.office.d")), node("st", "bucket", t("relay.storage"), t("relay.storage.d")), node("ho", "home", t("relay.home"), t("relay.home.d")),
      h("span", { class: "rlink l1" }, h("span", { class: "ln" })), h("span", { class: "rlink l2" }, h("span", { class: "ln" }))));
}

export function checkRow(r) {
  const st = { pass: ["ok", "check"], warn: ["warn", "warn"], fail: ["bad", "x"], info: ["ok", "check"], skip: ["wait", "more"], running: ["wait", null] }[r.status] || ["wait", null];
  return h("div", { class: "chk" },
    h("span", { class: `st ${st[0]}` }, st[1] ? icon(st[1]) : h("span", { class: "spin" })),
    h("div", {}, h("div", { class: "nm" }, r.label), h("div", { class: "dt" }, r.message), r.hint ? h("div", { class: "fix" }, r.hint) : null),
    h("span", { class: "ms" }, r.duration_ms != null ? `${r.duration_ms} ms` : ""));
}

function printKey(rk) {
  const w = window.open("", "_blank", "width=600,height=400");
  if (!w) return window.print();
  w.document.write(`<pre style="font:18px monospace;padding:40px">Workroam recovery key\n\n${rk.replace(/[<>&]/g, "")}</pre>`);
  w.document.close();
  w.print();
}

// QR scanning: camera frames are decoded in Rust (rqrr).
let stream;
async function scanQr(video, found) {
  try {
    stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" } });
  } catch (e) {
    return showError(new Error(t("camera_denied")));
  }
  video.srcObject = stream;
  video.hidden = false;
  await video.play();
  const c = document.createElement("canvas");
  const tick = async () => {
    if (!stream) return;
    const w = (c.width = Math.min(640, video.videoWidth || 640));
    const hh = (c.height = Math.round(w * ((video.videoHeight || 480) / (video.videoWidth || 640))));
    const g = c.getContext("2d", { willReadFrequently: true });
    g.drawImage(video, 0, 0, w, hh);
    const txt = await decode(g.getImageData(0, 0, w, hh));
    if (txt) { stopScan(video); found(txt); return; }
    setTimeout(tick, 250);
  };
  tick();
}

function stopScan(video) {
  stream?.getTracks().forEach((t) => t.stop());
  stream = null;
  video.hidden = true;
}

async function decode(img) {
  const luma = new Array(img.width * img.height);
  for (let i = 0, j = 0; i < img.data.length; i += 4, j++) luma[j] = (img.data[i] * 299 + img.data[i + 1] * 587 + img.data[i + 2] * 114) / 1000 | 0;
  return native("decode_qr", { width: img.width, height: img.height, luma });
}

function qrFromImage(found) {
  const f = h("input", { type: "file", accept: "image/*", hidden: true });
  f.addEventListener("change", async () => {
    const file = f.files[0];
    if (!file) return;
    const bmp = await createImageBitmap(file);
    const c = document.createElement("canvas");
    const scale = Math.min(1, 1200 / Math.max(bmp.width, bmp.height));
    c.width = Math.round(bmp.width * scale);
    c.height = Math.round(bmp.height * scale);
    const g = c.getContext("2d");
    g.drawImage(bmp, 0, 0, c.width, c.height);
    const txt = await decode(g.getImageData(0, 0, c.width, c.height));
    txt ? found(txt) : toast(t("qr_none"), "warn");
  });
  document.body.append(f);
  f.click();
  setTimeout(() => f.remove(), 60000);
}

export { sheet, acts };
