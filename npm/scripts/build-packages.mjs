#!/usr/bin/env node
// Assembles one npm release package with the launcher and all supported native binaries.
//
// Usage: node build-packages.mjs --version 0.1.0 --binaries <dir> --out <dir>
// <dir> holds one folder per Rust target (for example x86_64-apple-darwin/aether-motion).
import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const PLATFORMS = [
  { key: "darwin-arm64", target: "aarch64-apple-darwin", exe: "aether-motion" },
  { key: "darwin-x64", target: "x86_64-apple-darwin", exe: "aether-motion" },
  { key: "linux-x64", target: "x86_64-unknown-linux-gnu", exe: "aether-motion" },
  { key: "win32-x64", target: "x86_64-pc-windows-msvc", exe: "aether-motion.exe" },
];

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, a, i, all) => (a.startsWith("--") ? [...pairs, [a.slice(2), all[i + 1]]] : pairs), []),
);
for (const name of ["version", "binaries", "out"]) if (!args[name]) throw new Error(`missing --${name}`);
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(args.version)) throw new Error(`invalid version: ${args.version}`);

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const out = resolve(args.out);
rmSync(out, { recursive: true, force: true });

const main = join(out, "cli");
cpSync(join(root, "npm", "cli"), main, { recursive: true });
copyFileSync(join(root, "LICENSE"), join(main, "LICENSE"));

for (const platform of PLATFORMS) {
  const source = join(resolve(args.binaries), platform.target, platform.exe);
  if (!existsSync(source)) throw new Error(`missing binary for ${platform.key}: ${source}`);
  const dir = join(main, "bin", platform.key);
  mkdirSync(dir, { recursive: true });
  const binary = join(dir, platform.exe);
  copyFileSync(source, binary);
  chmodSync(binary, 0o755);
}

const manifest = JSON.parse(readFileSync(join(main, "package.json"), "utf8"));
manifest.version = args.version;
writeFileSync(join(main, "package.json"), JSON.stringify(manifest, null, 2) + "\n");
chmodSync(join(main, "bin", "aether-motion.js"), 0o755);
