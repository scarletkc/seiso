#!/usr/bin/env node
"use strict";

const { spawnSync } = require("node:child_process");
const supported = ["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64", "win32-arm64", "win32-x64"];
let platform = `${process.platform}-${process.arch}`;
if (!supported.includes(platform)) {
  console.error(`seiso: no prebuilt binary for ${platform}; install from source with cargo install seiso.`);
  process.exit(2);
}
// Node reports the glibc version it runs against; musl-based systems such as Alpine have none.
if (process.platform === "linux" && !process.report.getReport().header.glibcVersionRuntime) {
  platform += "-musl";
}
const name = `@scarletkc/seiso-${platform}`;
let binary;
try {
  binary = require.resolve(`${name}/${process.platform === "win32" ? "seiso.exe" : "seiso"}`);
} catch {
  console.error(
    `seiso: ${name} is not installed. It is an optional dependency of @scarletkc/seiso, so reinstall ` +
    "without --omit=optional or --no-optional, or install from source with cargo install seiso.",
  );
  process.exit(2);
}
const result = spawnSync(binary, process.argv.slice(2), {
  stdio: "inherit",
});
if (result.error) {
  console.error(`seiso: cannot start ${binary}: ${result.error.message}.`);
  process.exit(2);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
} else {
  process.exit(result.status ?? 2);
}
