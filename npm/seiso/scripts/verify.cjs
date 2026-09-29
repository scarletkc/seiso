"use strict";

const fs = require("node:fs");
const path = require("node:path");
const root = path.join(__dirname, "..");
const metadata = require(path.join(root, "package.json"));
const pins = Object.entries(metadata.optionalDependencies ?? {});
if (pins.length === 0 || pins.some(([, version]) => version !== metadata.version)) {
  throw new Error(`Every platform package must be pinned to ${metadata.version}.`);
}
for (const file of ["LICENSE", "README.md"]) {
  fs.accessSync(path.join(root, file));
}
