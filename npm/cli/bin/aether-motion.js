#!/usr/bin/env node
// Launcher: finds the prebuilt native binary from the matching optional platform package
// (@aether-motion/cli-<platform>-<arch>) and runs it. No install scripts, so it works with
// npm, pnpm, yarn, and bun without extra approval steps.
"use strict";

const { spawnSync } = require("node:child_process");

const PACKAGE = "@aether-motion/cli";
const key = `${process.platform}-${process.arch}`;
const supported = ["darwin-arm64", "darwin-x64", "linux-x64", "win32-x64"];

function findBinary() {
  if (process.env.AETHER_MOTION_BIN) return process.env.AETHER_MOTION_BIN;
  const exe = process.platform === "win32" ? "aether-motion.exe" : "aether-motion";
  try {
    return require.resolve(`${PACKAGE}-${key}/bin/${exe}`);
  } catch {
    const why = supported.includes(key)
      ? `the platform package ${PACKAGE}-${key} is missing (was it installed with --no-optional or --omit=optional?)`
      : `${key} has no prebuilt binary (supported: ${supported.join(", ")})`;
    console.error(`aether-motion: ${why}.`);
    console.error("Install a binary from https://github.com/ferxalb/aether-motion/releases or set AETHER_MOTION_BIN.");
    process.exit(1);
  }
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
