"use strict";

const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const root = path.join(__dirname, "..");
const metadata = require(path.join(root, "package.json"));
const manifest = JSON.parse(fs.readFileSync(path.join(root, "native", "manifest.json"), "utf8"));
if (metadata.version !== manifest.version) {
  throw new Error("The native binaries and npm package have different versions.");
}
for (const file of [
  "darwin-arm64/seiso",
  "darwin-x64/seiso",
  "linux-arm64/seiso",
  "linux-x64/seiso",
  "win32-x64/seiso.exe",
]) {
  const bytes = fs.readFileSync(path.join(root, "native", file));
  const digest = crypto.createHash("sha256").update(bytes).digest("hex");
  if (manifest.sha256[file] !== digest) {
    throw new Error(`Missing or mismatched binary checksum: ${file}`);
  }
}
for (const file of ["LICENSE", "README.md"]) {
  fs.accessSync(path.join(root, file));
}
