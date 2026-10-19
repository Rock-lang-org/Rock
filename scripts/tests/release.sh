#!/usr/bin/env bash
# Exercise the actual published bytes offline, replacing only the HTTP transport.
set -euo pipefail
if [[ ${0##*/} == curl ]]; then
    output= url=
    while [[ $# -gt 0 ]]; do
        case $1 in
            --output) output=$2; shift ;;
            https://*) url=$1 ;;
        esac
        shift
    done
    base=https://github.com/Rock-lang-org/Rock/releases
    case $url in
        "$base/latest") printf '%s/tag/%s' "$base" "$RELEASE_TEST_TAG"; exit 0 ;;
        "$base/download/$RELEASE_TEST_TAG/"*|"$base/latest/download/"*)
            if [[ -n $output ]]; then
                cp -- "$RELEASE_TEST_ASSETS/${url##*/}" "$output"
            else
                cat -- "$RELEASE_TEST_ASSETS/${url##*/}"
            fi ;;
        *) printf 'Unexpected offline request: %s\n' "$url" >&2; exit 1 ;;
    esac
    exit 0
fi

[[ $# == 1 ]] || { echo 'Usage: bash scripts/tests/release.sh dist/vVERSION' >&2; exit 1; }
export RELEASE_TEST_ASSETS
RELEASE_TEST_ASSETS=$(cd "$1" && pwd)
export RELEASE_TEST_TAG=${RELEASE_TEST_ASSETS##*/}
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
mkdir -p "$stage/bin" "$stage/home" "$stage/downloads" "$stage/project"
cp -- "$0" "$stage/bin/curl"
chmod 755 "$stage/bin/curl"
unset CARGO_TARGET_DIR ROCK_SYSROOT ROCKC ROCKUP_TOOLCHAIN
export HOME=$stage/home ROCKUP_HOME=$stage/home/.rockup SHELL=/bin/sh
export PATH=$stage/bin:$PATH TMPDIR=$stage/downloads
cd "$stage/project"
curl --proto '=https' -fsSL https://github.com/Rock-lang-org/Rock/releases/latest/download/install.sh | sh
# The bootstrap's temporary manager must be gone before invoking any shim.
[[ -z $(ls -A "$stage/downloads") ]]
manager=$ROCKUP_HOME/bin/rockup
[[ -d "$ROCKUP_HOME/toolchains/stable" ]]
[[ $(cat "$ROCKUP_HOME/default-toolchain") == stable ]]
"$manager" list
"$manager" update
"$manager" install "$RELEASE_TEST_TAG"
"$manager" default "$RELEASE_TEST_TAG"
"$manager" self update
"$manager" --version
"$ROCKUP_HOME/bin/rock" --version
"$ROCKUP_HOME/bin/rock-lsp" --help | grep -F 'Usage: rock-lsp'
printf '[crate]\nname = "release_smoke"\nversion = "0.1.0"\n\n[lib]\npath = "main.rk"\n' > rock.toml
printf 'main = ->\n    "Hello, Rock!".println!\n    0\n' > main.rk
"$ROCKUP_HOME/bin/rock" run | grep -Fx 'Hello, Rock!'
# An installed compiler must ignore an unrelated checkout's stale dev sysroot.
workspace=$stage/workspace
target=x86_64-unknown-linux-gnu
decoy=$workspace/target/lib/rocklib/$target
mkdir -p "$decoy" "$workspace/test_projects/app"
cp rock.toml main.rk "$workspace/test_projects/app/"
cp "$ROCKUP_HOME/toolchains/$RELEASE_TEST_TAG/lib/rocklib/$target/"{manifest,components}.json "$decoy/"
printf 'stale checkout artifact\n' > "$decoy/std.rkca"
printf 'stale checkout object\n' > "$decoy/std.o"
(
    cd "$workspace/test_projects/app"
    "$ROCKUP_HOME/bin/rock" run | grep -Fx 'Hello, Rock!'
    CARGO_TARGET_DIR="$workspace/target" "$ROCKUP_HOME/bin/rock" run | grep -Fx 'Hello, Rock!'
)
grep -Fx 'stale checkout artifact' "$decoy/std.rkca"
"$manager" remove stable
printf 'Release bootstrap, update, pin, self-update, shims and compile passed.\n'
