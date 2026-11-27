// Builds the `tunwarden` CLI for the app's target and puts it where Tauri's
// `externalBin` expects it: src-tauri/binaries/tunwarden-<target triple>.
// Tauri copies it next to the app's own binary, where the app looks for it
// (docs/desktop.md D2). Run by `beforeBuildCommand` in tauri.bundle.conf.json.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "../../..");
const target =
  process.env.TAURI_ENV_TARGET_TRIPLE || execFileSync("rustc", ["--print", "host-tuple"]).toString().trim();
const debug = process.env.TAURI_ENV_DEBUG === "true";
const ext = target.includes("windows") ? ".exe" : "";

const args = ["build", "-p", "tunwarden-cli", "--target", target, "--locked"];
if (!debug) args.push("--release");
console.log(`sidecar: cargo ${args.join(" ")}`);
execFileSync("cargo", args, { cwd: root, stdio: "inherit" });

const targetDir = process.env.CARGO_TARGET_DIR || path.join(root, "target");
const built = path.join(targetDir, target, debug ? "debug" : "release", `tunwarden${ext}`);
const dest = path.join(root, "apps/desktop/src-tauri/binaries", `tunwarden-${target}${ext}`);
fs.mkdirSync(path.dirname(dest), { recursive: true });
fs.copyFileSync(built, dest);
fs.chmodSync(dest, 0o755);
console.log(`sidecar: ${path.relative(root, dest)}`);
