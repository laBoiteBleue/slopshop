// Builds the helper executables (slopshop-raw, slopshop-ai) for the target being bundled and
// copies them where Tauri's `bundle.externalBin` expects them:
// `src-tauri/binaries/<name>-<target triple>[.exe]`. Tauri installs them next to the app's
// executable, where the app looks for them (ADR 0023, ADR 0025).
//
// Run by `beforeBuildCommand` in tauri.bundle.conf.json, which sets TAURI_ENV_TARGET_TRIPLE
// and TAURI_ENV_DEBUG.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const app = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const workspace = resolve(app, "..");
const targetDir = process.env.CARGO_TARGET_DIR ?? join(workspace, "target");

const host = /^host: (.+)$/m.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))?.[1];
const triple = process.env.TAURI_ENV_TARGET_TRIPLE ?? host;
if (!triple) throw new Error("cannot tell the target triple");
const debug = process.env.TAURI_ENV_DEBUG === "true";
const exe = triple.includes("windows") ? ".exe" : "";

// Building for the host without `--target` shares `target/release` with the rest of the
// workspace, so nothing is compiled twice.
const crossing = triple !== host;
const out = join(targetDir, ...(crossing ? [triple] : []), debug ? "debug" : "release");

const helpers = [
  { name: "slopshop-raw", features: [] },
  { name: "slopshop-ai", features: ["helper"] },
];

const binaries = join(app, "src-tauri", "binaries");
mkdirSync(binaries, { recursive: true });
for (const { name, features } of helpers) {
  const args = ["build", "--locked", "-p", name];
  if (!debug) args.push("--release");
  if (crossing) args.push("--target", triple);
  if (features.length) args.push("--features", features.join(","));
  execFileSync("cargo", args, { cwd: workspace, stdio: "inherit" });
  copyFileSync(join(out, name + exe), join(binaries, `${name}-${triple}${exe}`));
}
