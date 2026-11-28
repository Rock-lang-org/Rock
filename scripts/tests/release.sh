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
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
    Darwin-arm64) target=aarch64-apple-darwin ;;
    Darwin-x86_64) target=x86_64-apple-darwin ;;
    MINGW64_NT*-x86_64 | MSYS_NT*-x86_64) target=x86_64-pc-windows-msvc ;;
    *) echo "Unsupported host $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac
exe=
[[ $target == *-windows-* ]] && exe=.exe
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
mkdir -p "$stage/bin" "$stage/home" "$stage/downloads" "$stage/project"
if [[ -n $exe ]]; then
    # rockup.exe and install.ps1 run curl.exe, which cannot be a bash script.
    rustc --edition 2021 -O -o "$stage/bin/curl.exe" "$(dirname -- "$0")/fake_curl.rs"
else
    cp -- "$0" "$stage/bin/curl"
    chmod 755 "$stage/bin/curl"
fi
unset CARGO_TARGET_DIR ROCK_SYSROOT ROCKC ROCKUP_TOOLCHAIN
export HOME=$stage/home ROCKUP_HOME=$stage/home/.rockup SHELL=/bin/sh
export PATH=$stage/bin:$PATH TMPDIR=$stage/downloads TEMP=$stage/downloads TMP=$stage/downloads
# Only throwaway CI runners may have their Windows user PATH changed.
[[ -n $exe && ${GITHUB_ACTIONS-} != true ]] && export ROCKUP_NO_MODIFY_PATH=1
cd "$stage/project"
if [[ -n $exe ]]; then
    powershell -NoProfile -ExecutionPolicy Bypass -File "$(cygpath -w "$RELEASE_TEST_ASSETS/install.ps1")"
else
    curl --proto '=https' -fsSL https://github.com/Rock-lang-org/Rock/releases/latest/download/install.sh | sh
fi
# The bootstrap's temporary manager must be gone before invoking any shim.
[[ -z $(ls -A "$stage/downloads") ]]
manager=$ROCKUP_HOME/bin/rockup$exe
if [[ -n $exe && -z ${ROCKUP_NO_MODIFY_PATH-} ]]; then
    # New terminals find the proxies through the user PATH.
    powershell -NoProfile -Command "[Environment]::GetEnvironmentVariable('Path', 'User')" |
        tr -d '\r' | tr ';' '\n' | grep -Fxi "$(cygpath -w "$ROCKUP_HOME/bin")"
fi
[[ -d "$ROCKUP_HOME/toolchains/stable" ]]
[[ $(cat "$ROCKUP_HOME/default-toolchain") == stable ]]
"$manager" list
"$manager" update
"$manager" install "$RELEASE_TEST_TAG"
"$manager" default "$RELEASE_TEST_TAG"
"$manager" self update
"$manager" --version
rock=$ROCKUP_HOME/bin/rock$exe
"$rock" --version
"$ROCKUP_HOME/bin/rock-lsp$exe" --help | grep -F 'Usage: rock-lsp'
printf '[crate]\nname = "release_smoke"\nversion = "0.1.0"\n\n[[bin]]\npath = "main.rk"\n' > rock.toml
printf 'main = !->\n    "Hello, Rock!".println!\n' > main.rk
"$rock" run | grep -Fx 'Hello, Rock!'
# An installed compiler must ignore an unrelated checkout's stale dev sysroot.
workspace=$stage/workspace
decoy=$workspace/target/lib/rocklib/$target
mkdir -p "$decoy" "$workspace/test_projects/app"
cp rock.toml main.rk "$workspace/test_projects/app/"
cp "$ROCKUP_HOME/toolchains/$RELEASE_TEST_TAG/lib/rocklib/$target/"{manifest,components}.json "$decoy/"
printf 'stale checkout artifact\n' > "$decoy/std.rkca"
printf 'stale checkout object\n' > "$decoy/std.o"
(
    cd "$workspace/test_projects/app"
    "$rock" run | grep -Fx 'Hello, Rock!'
    CARGO_TARGET_DIR="$workspace/target" "$rock" run | grep -Fx 'Hello, Rock!'
)
grep -Fx 'stale checkout artifact' "$decoy/std.rkca"
"$manager" remove stable
printf 'Release bootstrap, update, pin, self-update, shims and compile passed.\n'
