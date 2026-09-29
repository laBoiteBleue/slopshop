import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";

// Desktop app, not a web page: no native webview context menu in production builds. Dev builds
// keep it for debugging (Inspect Element). Custom menus call preventDefault themselves.
if (!import.meta.env.DEV) {
  window.addEventListener("contextmenu", (e) => e.preventDefault());
}

const target = document.getElementById("app");
if (!target) throw new Error("missing #app element");

export default mount(App, { target });
