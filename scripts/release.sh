#!/usr/bin/env bash
# Build one matched compiler/std distribution; uploading is explicitly opt-in.
set -euo pipefail
export LC_ALL=C
unset TAR_OPTIONS

die() { printf 'release: %s\n' "$*" >&2; exit 1; }
[[ $# -ge 1 && $# -le 2 ]] || die 'Usage: bash scripts/release.sh vVERSION [--publish]'
tag=$1
[[ $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$ ]] || die 'Expected vMAJOR.MINOR.PATCH[-PRERELEASE]'
publish=${2-}
[[ -z $publish || $publish == --publish ]] || die 'Unknown option'
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
notes=$root/docs/release-notes/$tag.md
[[ -s $notes ]] || die "Missing release notes: $notes"
target=x86_64-unknown-linux-gnu
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || die "Only $target is supported"
for tool in cargo cc tar gzip sha256sum git ldd; do
    command -v "$tool" >/dev/null || die "Missing command: $tool"
done
if [[ -n ${LLVM_SYS_180_PREFIX:-} ]]; then
    llvm_config=$LLVM_SYS_180_PREFIX/bin/llvm-config
else
    llvm_config=$(command -v llvm-config-18 || command -v llvm-config) || die 'Install LLVM 18 development files and set LLVM_SYS_180_PREFIX'
fi
[[ $("$llvm_config" --version) == 18.* ]] || die 'Release builds require LLVM 18; set LLVM_SYS_180_PREFIX to its installation'
"$llvm_config" --link-static --libfiles >/dev/null || die 'LLVM 18 static archives are required (Ubuntu: llvm-18-dev libpolly-18-dev)'
export LLVM_SYS_180_PREFIX
LLVM_SYS_180_PREFIX=$("$llvm_config" --prefix)
# Keep --version output consistent with the release tag.
for package in rock rockc rockup rock-lsp lib rock-shared; do
    version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' "$package/Cargo.toml" | head -n 1)
    [[ $version == "${tag#v}" ]] || die "$package/Cargo.toml version $version does not match $tag"
done
if [[ $publish == --publish ]]; then
    command -v gh >/dev/null || die 'Publishing requires authenticated GitHub CLI (gh)'
    [[ -z $(git status --porcelain --untracked-files=normal) ]] || die 'Publishing requires a clean checkout'
    [[ $(git rev-parse "refs/tags/$tag^{commit}") == "$(git rev-parse HEAD)" ]] || die 'Release tag must point at HEAD'
    remote_tag=$(git ls-remote https://github.com/Rock-lang-org/Rock.git "refs/tags/$tag" "refs/tags/$tag^{}")
    remote_commit=$(printf '%s\n' "$remote_tag" | awk 'NR == 1 { commit = $1 } /\^\{\}$/ { commit = $1 } END { print commit }')
    [[ $remote_commit == "$(git rev-parse HEAD)" ]] || die 'Tag must already exist at HEAD on GitHub'
fi

out=$root/dist/$tag
[[ ! -e $out ]] || die "$out already exists; choose a fresh output directory by moving the previous build"
mkdir -p "$root/dist"
stage=$(mktemp -d "${TMPDIR:-/tmp}/rock-release.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT
# An explicit target and target directory prevent ambient Cargo settings from
# selecting a different architecture or packaging stale build output.
export CARGO_TARGET_DIR=$root/target
cargo build --locked --release --target "$target" -p rock -p rockc -p rockup -p rock-lsp
build=$CARGO_TARGET_DIR/$target/release
# Check transitive dependencies too: a build-host LLVM must never hide a
# missing end-user dependency in a release that appears to run locally.
for binary in rock rockc rockup rock-lsp; do
    dependencies=$(ldd "$build/$binary") || die "Cannot inspect runtime dependencies of $binary"
    if grep -Ei 'lib(LLVM|clang)|not found' <<< "$dependencies"; then
        die "$binary has a shared LLVM dependency or an unresolved runtime library"
    fi
done
toolchain=$stage/toolchain
mkdir -p "$toolchain/bin" "$toolchain/share/licenses/llvm" "$stage/assets"
install -m 644 "$root"/licenses/llvm/*.txt "$toolchain/share/licenses/llvm/"
for binary in rock rockc rock-lsp; do
    install -m 755 "$build/$binary" "$toolchain/bin/$binary"
done
"$build/rockup" dev std package --path "$root/std" --sysroot "$toolchain" --copy-source --rockc "$build/rockc"
rm -rf -- "$toolchain/src/std/build" "$toolchain/src/std/target"
manifest=$toolchain/lib/rocklib/$target/manifest.json
sed -i "s/\"toolchain_version\": \"dev-[^\"]*\"/\"toolchain_version\": \"$tag\"/" "$manifest"

# Smoke-test the installed layout away from the checkout and all dev sysroots.
mkdir -p "$stage/home" "$stage/project"
printf '[crate]\nname = "release_smoke"\nversion = "0.1.0"\n\n[lib]\npath = "main.rk"\n' > "$stage/project/rock.toml"
printf 'main = ->\n    "Hello, Rock!".println!\n    0\n' > "$stage/project/main.rk"
(
    unset CARGO_TARGET_DIR ROCK_SYSROOT ROCKC ROCKUP_TOOLCHAIN
    export HOME=$stage/home ROCKUP_HOME=$stage/home/.rockup SHELL=/bin/sh
    cd "$stage/project"
    "$build/rockup" install "$tag" --path "$toolchain"
    "$ROCKUP_HOME/bin/rockup" default "$tag"
    "$ROCKUP_HOME/bin/rock" --version
    "$ROCKUP_HOME/bin/rockc" --version
    "$ROCKUP_HOME/bin/rock-lsp" --help > "$stage/lsp-help.log"
    grep -F 'Usage: rock-lsp' "$stage/lsp-help.log"
    "$ROCKUP_HOME/bin/rock" run > "$stage/smoke.log"
    grep -Fx 'Hello, Rock!' "$stage/smoke.log"
)

assets=$stage/assets
tar --format=ustar -czf "$assets/rock-$tag-$target.tar.gz" -C "$toolchain" bin lib src share
tar --format=ustar -czf "$assets/std-$tag-$target.tar.gz" -C "$toolchain/lib/rocklib/$target" .
install -m 755 "$build/rockup" "$assets/rockup-$target"
install -m 644 scripts/install.sh "$assets/install.sh"
(
    cd "$assets"
    for asset in *.tar.gz "rockup-$target" install.sh; do
        sha256sum "$asset" > "$asset.sha256"
        sha256sum --check "$asset.sha256"
    done
)
mv "$assets" "$out"
printf 'Packaged and smoke-tested %s\n' "$out"
if [[ $publish == --publish ]]; then
    gh release create "$tag" "$out"/* --repo Rock-lang-org/Rock --verify-tag --draft \
        --title "Rock $tag" --notes-file "$notes"
fi
