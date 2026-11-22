#!/usr/bin/env node
// Runs the bundled native binary for the current Node platform and architecture.
"use strict";

const { spawnSync } = require("node:child_process");
const { join } = require("node:path");

const PACKAGE = "@aether-motion/cli";
const key = `${process.platform}-${process.arch}`;
const supported = ["darwin-arm64", "darwin-x64", "linux-x64", "win32-x64"];

function findBinary() {
  if (process.env.AETHER_MOTION_BIN) return process.env.AETHER_MOTION_BIN;
  if (!supported.includes(key)) {
    console.error(`aether-motion: ${key} has no prebuilt binary (supported: ${supported.join(", ")}).`);
    console.error("Install a binary from https://github.com/ferxalb/aether-motion/releases or set AETHER_MOTION_BIN.");
    process.exit(1);
  }
  const exe = process.platform === "win32" ? "aether-motion.exe" : "aether-motion";
  return join(__dirname, key, exe);
}

const result = spawnSync(findBinary(), process.argv.slice(2), {
  stdio: "inherit",
  env: { ...process.env, AETHER_MOTION_NPM_PACKAGE: PACKAGE },
});
if (result.error) {
  console.error(`aether-motion: could not start the binary: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) process.kill(process.pid, result.signal);
process.exit(result.status ?? 1);
