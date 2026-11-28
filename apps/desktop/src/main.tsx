import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import { App } from "./App";
import { TrayPopover } from "./components/TrayPopover";

// Follow the system's light or dark setting.
const dark = window.matchMedia("(prefers-color-scheme: dark)");
const theme = () => document.documentElement.classList.toggle("dark", dark.matches);
theme();
dark.addEventListener("change", theme);
// In the app on macOS and Windows the window has a native translucent frame
// (tauri.macos.conf.json, tauri.windows.conf.json); on macOS the traffic
// lights sit inside the sidebar. A browser or Linux gets an opaque page.
const os = /Mac/.test(navigator.userAgent) ? "macos" : /Windows/.test(navigator.userAgent) ? "windows" : "linux";
document.documentElement.dataset.os = os;
if ("__TAURI_INTERNALS__" in window && os !== "linux") document.documentElement.dataset.frame = "native";
// The tray popover is a second window on the same page (tauri.conf.json).
const popover = location.hash === "#popover";
if (popover) document.documentElement.dataset.window = "popover";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {popover ? <TrayPopover /> : <App />}
  </StrictMode>,
);
