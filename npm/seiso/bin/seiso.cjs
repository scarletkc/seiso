#!/usr/bin/env node
"use strict";

const path = require("node:path");
const { spawnSync } = require("node:child_process");
const binaries = {
  "darwin-arm64": "darwin-arm64/seiso",
  "darwin-x64": "darwin-x64/seiso",
  "linux-arm64": "linux-arm64/seiso",
  "linux-x64": "linux-x64/seiso",
  "win32-x64": "win32-x64/seiso.exe",
};
const platform = `${process.platform}-${process.arch}`;
const binary = binaries[platform];
if (!binary) {
  console.error(`seiso: no bundled binary for ${platform}; install from source with cargo install seiso.`);
  process.exit(2);
}
const result = spawnSync(path.join(__dirname, "..", "native", binary), process.argv.slice(2), {
  stdio: "inherit",
});
if (result.error) {
  // Without the glibc loader, as on Alpine, starting a bundled Linux binary fails with ENOENT.
  const hint = process.platform === "linux" && result.error.code === "ENOENT"
    ? "; the bundled Linux binaries need glibc, so on musl-based systems such as Alpine install from source with cargo install seiso"
    : "";
  console.error(`seiso: cannot start the bundled binary: ${result.error.message}${hint}.`);
  process.exit(2);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
} else {
  process.exit(result.status ?? 2);
}
