import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { hasShortcutModifier, isMac } from "./lib/platform";

// Desktop app, not a web page (see CLAUDE.md).

// No native webview context menu. Custom menus call preventDefault themselves. For debugging,
// dev builds open the native menu (Inspect Element) on Shift+right-click; Ctrl+Shift+I
// (Cmd+Option+I on macOS) opens the devtools too.
window.addEventListener("contextmenu", (e) => {
  if (import.meta.env.DEV && e.shiftKey) return;
  e.preventDefault();
});

// No browser shortcuts in production (reload, print, find, caret browsing, history). A page
// handler that calls preventDefault stops WebView2/WebKit from running them. Dev builds keep
// them (F5 reloads the Vite page). preventDefault does not stop propagation: app shortcuts
// such as Ctrl+Z still run. On macOS only Cmd shortcuts are browser shortcuts: Ctrl+letter
// and Option+Arrow are Cocoa caret movements in text fields and must keep working.
if (!import.meta.env.DEV) {
  const blockedKeys = new Set([
    "f3",
    "f5",
    "f7",
    "browserback",
    "browserforward",
    "browserrefresh",
  ]);
  const blockedWithModifier = new Set(["r", "p", "f", "g", "u", "s", "j"]);
  window.addEventListener(
    "keydown",
    (e) => {
      const key = e.key.toLowerCase();
      const historyKeys = !isMac && e.altKey && (key === "arrowleft" || key === "arrowright");
      if (
        blockedKeys.has(key) ||
        (hasShortcutModifier(e) && blockedWithModifier.has(key)) ||
        historyKeys
      ) {
        e.preventDefault();
      }
    },
    { capture: true },
  );
}

const target = document.getElementById("app");
if (!target) throw new Error("missing #app element");

export default mount(App, { target });
