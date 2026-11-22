# Releasing

A release is a `v<version>` tag that matches `Cargo.toml`. Update `Cargo.toml`,
`Cargo.lock`, and `CHANGELOG.md`, commit the release changes, and push the branch
and matching tag. The Release workflow builds and tests the native binaries,
creates the GitHub release, and stages the npm package for maintainer approval.

## npm package

Install `@aether-motion/cli`. The package bundles the native executables for
Apple Silicon and Intel macOS, x64 Linux (glibc), and x64 Windows. On launch, the
Node shim selects the executable using Node's platform and architecture. Each
install downloads the full archive, which works offline and requires no install
scripts. Set `AETHER_MOTION_BIN` to use a different native executable.

The macOS job also combines its two builds with `lipo` for the universal download
on GitHub. npm bundles the individual macOS builds alongside Linux and Windows.

## Configure npm Trusted Publishing

On npmjs.com, open **Settings → Trusted Publisher** for `@aether-motion/cli` and
select **GitHub Actions**.

| Setting | Value |
| --- | --- |
| Organization or user | `ferxalb` |
| Repository | `aether-motion` |
| Workflow filename | `release.yml` |
| Environment | Leave empty |
| Allow `npm publish` | Leave unchecked |
| Allow `npm dist-tag` | Leave unchecked |

The npm job uses OIDC with `id-token: write` and stages the package on
`https://registry.npmjs.org` without an npm token secret. Use the connection
within two days of creating it to validate it. After the workflow stages a
release, approve it with 2FA in **Staged Packages** on npmjs.com. Stable versions
use `latest` and prereleases use `next`.

## Stage manually

If staging fails after the GitHub release is created, download the
`npm-tarballs` artifact from that release run. Use Node 22.14 or newer and npm
11.15 or newer. From the extracted artifact directory, set the version to the
one in the tarball filename.

```sh
npm login --registry=https://registry.npmjs.org
version=0.2.1
tag=latest
case "$version" in *-*) tag=next ;; esac
npm stage publish "./aether-motion-cli-$version.tgz" --access public --tag "$tag"
```

The `./` prefix makes npm resolve the argument as a local file. Without it, a
relative path containing a directory can be interpreted as GitHub shorthand
and trigger an SSH lookup. After approval, verify the install with
`npx --yes @aether-motion/cli@"$version" --version`.
