#!/usr/bin/env node
// Assembles the npm packages for a release: one platform package per target holding the native
// binary, and the `@aether-motion/cli` launcher that depends on them (exact versions).
//
// Usage: node build-packages.mjs --version 0.1.0 --binaries <dir> --out <dir>
// <dir> holds one folder per Rust target (for example x86_64-apple-darwin/aether-motion).
import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const PLATFORMS = [
  { key: "darwin-arm64", target: "aarch64-apple-darwin", os: "darwin", cpu: "arm64", exe: "aether-motion" },
  { key: "darwin-x64", target: "x86_64-apple-darwin", os: "darwin", cpu: "x64", exe: "aether-motion" },
  { key: "linux-x64", target: "x86_64-unknown-linux-gnu", os: "linux", cpu: "x64", exe: "aether-motion" },
  { key: "win32-x64", target: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64", exe: "aether-motion.exe" },
];

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, a, i, all) => (a.startsWith("--") ? [...pairs, [a.slice(2), all[i + 1]]] : pairs), []),
);
for (const name of ["version", "binaries", "out"]) if (!args[name]) throw new Error(`missing --${name}`);
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(args.version)) throw new Error(`invalid version: ${args.version}`);

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const out = resolve(args.out);
rmSync(out, { recursive: true, force: true });

const repository = { type: "git", url: "git+https://github.com/ferxalb/aether-hyperframes.git" };
const optionalDependencies = {};

for (const p of PLATFORMS) {
  const source = join(resolve(args.binaries), p.target, p.exe);
  if (!existsSync(source)) throw new Error(`missing binary for ${p.key}: ${source}`);
  const name = `@aether-motion/cli-${p.key}`;
  const dir = join(out, `cli-${p.key}`);
  mkdirSync(join(dir, "bin"), { recursive: true });
  copyFileSync(source, join(dir, "bin", p.exe));
  chmodSync(join(dir, "bin", p.exe), 0o755);
  copyFileSync(join(root, "LICENSE"), join(dir, "LICENSE"));
  writeFileSync(
    join(dir, "package.json"),
    JSON.stringify(
      {
        name,
        version: args.version,
        description: `AETHER Motion native binary for ${p.os} ${p.cpu}. Install @aether-motion/cli instead.`,
        license: "Apache-2.0",
        os: [p.os],
        cpu: [p.cpu],
        files: ["bin", "LICENSE"],
        repository,
        publishConfig: { access: "public" },
      },
      null,
      2,
    ) + "\n",
  );
  optionalDependencies[name] = args.version;
}

const main = join(out, "cli");
cpSync(join(root, "npm", "cli"), main, { recursive: true });
copyFileSync(join(root, "LICENSE"), join(main, "LICENSE"));
const manifest = JSON.parse(readFileSync(join(main, "package.json"), "utf8"));
manifest.version = args.version;
manifest.optionalDependencies = optionalDependencies;
writeFileSync(join(main, "package.json"), JSON.stringify(manifest, null, 2) + "\n");
chmodSync(join(main, "bin", "aether-motion.js"), 0o755);

// Platform packages first so the launcher never points at a version that is not published yet.
console.log([...PLATFORMS.map((p) => `cli-${p.key}`), "cli"].join("\n"));
