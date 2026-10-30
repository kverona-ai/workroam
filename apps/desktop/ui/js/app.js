// Entry point for every window. The hash picks the window: #cards for the
// corner cards, #details/<approval id> for an approval, otherwise the main
// window (the setup wizard until this computer is set up).
import { setLang } from "./i18n.js";
import { ipc, glassPointer, $, h, inTauri, fill } from "./core.js";
import { startSetup } from "./setup.js";
import { startMain, push, pull, go, S } from "./main.js";
import { startCards, startDetails } from "./cards.js";
import { trayActions } from "./tray.js";

const root = $("#root");
const route = location.hash.slice(1);

async function boot() {
  let info = null;
  try {
    info = await ipc("daemon.info");
    const lang = await ipc("config.get", { key: "ui.lang" }).catch(() => null);
    setLang(lang?.value || lang || "");
  } catch {
    setLang("");
  }
  glassPointer();
  if (route === "cards") return startCards(root);
  if (route.startsWith("details/")) return startDetails(root, route.slice(8));
  if (!inTauri) document.documentElement.classList.add("browser");
  trayActions({ push, pull, go, select: (n) => (S.sel = n) });
  if (!info) {
    fill(root, h("div", { class: "pad" }, h("p", {}, "Workroam background service is not running.")));
    setTimeout(() => location.reload(), 2000);
    return;
  }
  if (!info.configured) return startSetup(root, (opts) => startMain(root, opts));
  return startMain(root);
}

boot();
