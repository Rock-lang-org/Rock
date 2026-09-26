# Release Maintainer Guide

This guide defines the Linux release contract for `Rock-lang-org/Rock`. Releases use tags named `vVERSION`. Only `x86_64-unknown-linux-gnu` is supported, built on Ubuntu 24.04 (glibc 2.39 baseline) with statically linked LLVM 18. End users do not need to install LLVM, but still need a C linker, curl, and CA certificates (`build-essential curl ca-certificates` on Ubuntu 24.04), plus GNU tar, gzip, and `sha256sum`. These are not fully static executables: system-library dependencies remain, and archives do not bundle the operating-system runtime.

The rockup asset format starts with `v0.5.0`. Earlier releases provide historical standalone assets and are not supported by the bootstrap. This guide uses `v0.6.0` as its example; choose a new, unused tag when preparing subsequent releases.

## Repository Transfer

The canonical repository is now `Rock-lang-org/Rock`, and the book is published at <https://rock-lang-org.github.io/Rock/>. Pre-`v0.5.2` release assets still contain the old installer and binary download URLs; transferring the repository or editing source does not rewrite those assets. `v0.5.2` was the first published release built with the updated `rockup` and `scripts/install.sh` targeting the canonical repository after the transfer. Subsequent releases retain those canonical URLs and provide fresh assets and checksum sidecars.

An old `rockup self update` rejects a redirect to the canonical repository. Using the current published release (`v0.5.2` or later), affected users can recover without deleting installed toolchains:

1. Stop other `rockup` operations and move only `ROCKUP_HOME/bin/rockup` to an unused backup filename (the default home is `$HOME/.rockup`). Keep the toolchains, pins, and default selection intact.
2. Rerun the bootstrap from the canonical URL below, using the same `ROCKUP_HOME` if customized. The bootstrap intentionally refuses to overwrite an existing manager, so the backup step is required.
3. If `stable` is already installed, the bootstrap reports a toolchain-installation error after persisting the new manager. Run `"$ROCKUP_HOME/bin/rockup" update stable` (or `"$HOME/.rockup/bin/rockup" update stable` for the default home), then verify `rockup --version` and `rock --version`. Keep the backup until verification succeeds; if manager installation fails, restore it to its original path.

Do not bypass URL validation or checksum checks. Verify both fresh installation and migration from an old manager against the published assets. Pinned pre-`v0.5.2` releases still contain old managers even when downloaded through the new repository URL.

In the new organization, check GitHub Pages settings (GitHub Actions deployment source), organization Actions policies, workflow permissions, and the `github-pages` environment's protection rules and allowed deployment branches. Confirm a successful book deployment and the new public URL; repository transfer alone is not evidence that Pages or release workflows can deploy.

## Asset Contract

For `VERSION=0.6.0` and `TARGET=x86_64-unknown-linux-gnu`, attach:

| Asset | Contents or purpose |
| --- | --- |
| `rock-v0.6.0-x86_64-unknown-linux-gnu.tar.gz` | Complete toolchain, extracted directly into its toolchain root |
| `rockup-x86_64-unknown-linux-gnu` | Standalone executable, not an archive |
| `stdlib-v0.6.0-x86_64-unknown-linux-gnu.tar.gz` | Matching standard-library component archive |
| Each binary/archive name followed by `.sha256` | SHA-256 sidecar for that exact asset |
| `install.sh` | POSIX bootstrap from `scripts/install.sh` |

The complete toolchain contains `bin/rock`, `bin/rockc`, `bin/rock-lsp`, the matching standard-library artifacts and component metadata under `lib/rocklib/TARGET/`, standard-library sources under `src/stdlib/`, and LLVM license notices under `share/licenses/llvm/`. Keep the compiler, object files, serialized artifacts, and metadata from the same build. Do not add an enclosing version directory inside the archive.

Generate sidecars from the asset directory using `sha256sum ASSET > ASSET.sha256`. Each sidecar must contain exactly one checksum line naming the asset's basename, not a local path. The bootstrap accepts the standard text or binary sha256sum separator; unrelated filenames, extra lines, and mismatched hashes are rejected. These are integrity checks, not signed provenance.

The public bootstrap URL is:

```text
https://github.com/Rock-lang-org/Rock/releases/latest/download/install.sh
```

Stable standalone-manager downloads use `releases/latest/download/rockup-TARGET` and its `.sha256` sidecar. Pinned downloads use `releases/download/vVERSION/rockup-TARGET` and its sidecar. The bootstrap verifies the standalone manager and runs `rockup self install`, which copies the manager and command shims and adds shell setup. It then invokes the persisted manager by absolute path with `install stable` to resolve the latest non-prerelease tag and download the matching `rock-vVERSION-TARGET.tar.gz`. An optional `vVERSION` script argument pins both the manager and toolchain, using `install vVERSION` instead. No shell activation is needed between installation steps. Keep all required assets together before publishing.

## Local Packaging

The maintainer entry point is:

```sh
bash scripts/release.sh v0.6.0
```

Its contract is to build, package, and smoke-test locally, writing output to `dist/v0.6.0`. This default mode must not create tags, upload assets, or publish releases. Use Ubuntu 24.04 x86_64 with Rust/Cargo, LLVM 18 development files and static archives, a C toolchain, GNU tar, gzip, curl, CA certificates, and sha256sum. A build on a newer distribution can accidentally raise the glibc baseline; use the baseline environment for distributable artifacts.

Install the source-build dependencies and select LLVM 18 before packaging:

```sh
sudo apt install llvm-18-dev libpolly-18-dev libzstd-dev libxml2-dev zlib1g-dev libffi-dev libedit-dev libncurses-dev build-essential
export LLVM_SYS_180_PREFIX=/usr/lib/llvm-18
```

Static LLVM archives are mandatory; there is no dynamic-linking fallback. The new packaging contract requires rejecting shared LLVM dependencies via `ldd` and including LLVM license notices under `share/licenses/llvm/`.

Before tagging, synchronize the package versions in `rock/Cargo.toml`, `rockc/Cargo.toml`, `rockup/Cargo.toml`, `rock-lsp/Cargo.toml`, `lib/Cargo.toml`, and `rock-shared/Cargo.toml`, and refresh `Cargo.lock`; the script rejects mismatched CLI versions. Existing output directories are never overwritten. The separate stdlib archive extracts directly to a target component directory and includes its manifests, artifact, and object file.

Before accepting the output, inspect archive layouts, verify every sidecar from the output directory, and smoke-test with a disposable `HOME` and `ROCKUP_HOME`. Test that the bootstrap installs the manager, shims, shell setup, and selected toolchain in one invocation. Remove temporary downloads afterward: `rockup self install` must persist the downloaded manager at `ROCKUP_HOME/bin/rockup`. Activate the shell, then check `rock --version`, `rock-lsp --help`, and compilation/execution of a small application with the packaged stdlib without a separate install command. All three shims must work even though the bootstrap staging directory is gone. A shim pointing into the build or download staging directory is a release blocker.

Run the bootstrap checks without network access:

```sh
sh -n scripts/install.sh
sh scripts/tests/install.sh
bash scripts/tests/release.sh dist/v0.6.0
```

The final check uses the real packaged binaries and archives with an offline HTTP fixture to exercise bootstrap cleanup, stable updates, pinned installation, self-update, shims, and compilation. The new CI contract requires running package smoke tests before draft creation in a clean Ubuntu 24.04 container with the runtime packages listed above and no LLVM installation. A successful build on an LLVM-equipped runner alone does not validate the runtime contract.

## Draft Creation

Uploading is explicitly opt-in:

```sh
bash scripts/release.sh v0.6.0 --publish
```

Despite the option name, this creates a **draft only**, using authenticated `gh`. It requires the existing tag to point at local `HEAD` and to exist on GitHub; it must not create or move a tag. Arrange the reviewed commit and remote tag separately through the project's normal maintainer process. Do not use this option merely to test packaging.

The tag workflow for `v*` runs on Ubuntu 24.04, tests and packages the toolchain, and creates a draft release. It must not automatically make a public release. Coordinate the workflow and local `--publish` path rather than trying to create the same draft twice. Verify the implemented script/workflow and their logs before relying on this contract; documentation alone is not evidence of a successful packaging run.

## Review and Publish

1. Confirm the draft tag identifies the reviewed commit and the build used the supported baseline.
2. Review test and smoke-test logs, release notes, platform requirements, and known limitations.
3. Check that every required asset and exact checksum sidecar is attached and that no private files or build paths leaked into archives.
4. Download draft assets with authenticated maintainer access and verify them; anonymous bootstrap URLs cannot install a draft.
5. Manually publish the reviewed draft in GitHub. Mark actual prereleases as prereleases; stable must mean the latest non-prerelease release.
6. For the first new-format stable release, ensure GitHub marks it as Latest rather than leaving a historical release selected, then test the anonymous `latest/download/install.sh` flow and a pinned install in disposable homes.

No local packaging command or tag workflow should silently publish publicly. Do not claim Windows, macOS, nightly, automatic cross-target support, or Rustup feature parity. If the public smoke test fails, communicate the problem and correct the release deliberately rather than telling users to skip checksum checks.
