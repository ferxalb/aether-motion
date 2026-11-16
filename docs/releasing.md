# Releasing

A release is a `v<version>` tag that matches `Cargo.toml`. `.github/workflows/release.yml` builds the binaries for every target, attaches them to a GitHub release, and publishes the npm packages.

The macOS job builds both `aarch64-apple-darwin` and `x86_64-apple-darwin` on an Apple Silicon runner. It tests the Intel build through Rosetta and combines both executables with `lipo`. The GitHub release offers one macOS download, `aether-motion-v<version>-universal-apple-darwin.tar.gz`. npm uses the architecture-specific builds so each installation downloads only its own binary.

## npm packages

| Package | Contents |
| --- | --- |
| `@aether-motion/cli` | Node launcher (`bin/aether-motion.js`), no install scripts; depends on the platform packages as `optionalDependencies` with exact versions |
| `@aether-motion/cli-darwin-arm64`, `-darwin-x64`, `-linux-x64`, `-win32-x64` | The native binary for that platform (`os`/`cpu` fields make the package manager install only the matching one) |

`npm/scripts/build-packages.mjs` assembles them from the build artifacts. Using optional dependencies instead of a `postinstall` download means npm, pnpm, yarn, and bun all work without approving lifecycle scripts, and installs stay offline-friendly.

## First npm release

Create the `aether-motion` organization on npmjs.com and enable 2FA on your npm account. Use Node 22.14 or newer and npm 11.15 or newer.

Commit the release changes, then push the branch and the tag. For version 0.1.0:

```sh
git push origin main
git tag v0.1.0
git push origin v0.1.0
```

Wait for the Release workflow to finish. It builds the four native binaries, creates the GitHub release, and uploads the five npm tarballs as the `npm-tarballs` Actions artifact. While any of the five package names is missing from the public registry, the workflow leaves staging to a maintainer.

Download and extract the artifact, then run these commands from the directory containing the five tarballs:

```bash
npm login
version=0.1.0
tag=latest
case "$version" in *-*) tag=next ;; esac
for package in cli-darwin-arm64 cli-darwin-x64 cli-linux-x64 cli-win32-x64 cli; do
  npm stage publish "./aether-motion-$package-$version.tgz" --access public --tag "$tag" || break
done
```

In **Staged Packages** on npmjs.com, review and approve the four platform packages first, then approve `@aether-motion/cli`. All five need the same version. Verify the install with `npx --yes @aether-motion/cli@0.1.0 --version`.

## Subsequent npm releases

For each of the five packages, open **Settings → Trusted Publisher**, select **GitHub Actions**, and enter:

| Setting | Value |
| --- | --- |
| Organization or user | `ferxalb` |
| Repository | `aether-motion` |
| Workflow filename | `release.yml` |
| Environment | Leave empty |
| Allowed actions | Staged publishing |

Create these connections just before the next release. npm requires the first successful use within two days. The GitHub-hosted `npm` job uses OIDC with `id-token: write`. npm automatically adds provenance for public packages built from a public repository.

Bump the version in `Cargo.toml` and update `Cargo.lock` and `CHANGELOG.md`, then commit and push the matching tag. GitHub Actions stages the five packages. Approve them in npmjs.com in the same order as the first release. Stable versions use `latest` and pre-releases such as `0.2.0-rc.1` use `next`.
