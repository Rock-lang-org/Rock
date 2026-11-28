#!/bin/sh
# Offline bootstrap contract tests; no real downloads or user-home writes.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' 0
trap 'exit 1' HUP INT TERM
FIXTURES=$root/scripts/tests/fixtures
export FIXTURES
for script in "$root/scripts/install.sh" "$root/scripts/tests/install.sh" "$FIXTURES"/*; do
    sh -n "$script"
done
mkdir "$work/bin"
cp "$FIXTURES/curl" "$work/bin/curl"
cp "$FIXTURES/platform" "$work/bin/platform"
chmod 700 "$work/bin/curl" "$work/bin/platform"
for tool in uname getconf tar; do
    ln -s platform "$work/bin/$tool"
done
PATH=$work/bin:$PATH
export PATH

run_case() {
    name=$1
    expected=$2
    shift 2
    TEST_ROOT=$work/$name
    HOME=$TEST_ROOT/home
    ROCKUP_HOME=$HOME/custom\ rockup
    TMPDIR=$TEST_ROOT/tmp
    export TEST_ROOT HOME ROCKUP_HOME TMPDIR
    mkdir -p "$HOME" "$TMPDIR"
    case "$name" in
        existing)
            mkdir -p "$ROCKUP_HOME/bin"
            cp "$FIXTURES/rockup" "$ROCKUP_HOME/bin/rockup"
            ;;
        dangling)
            mkdir -p "$ROCKUP_HOME/bin"
            ln -s missing "$ROCKUP_HOME/bin/rockup"
            ;;
        default-home) unset ROCKUP_HOME ;;
        empty-home) ROCKUP_HOME= ;;
        relative-home) ROCKUP_HOME=relative ;;
    esac
    status=0
    sh "$root/scripts/install.sh" "$@" > "$TEST_ROOT/log" 2>&1 || status=$?
    if [ "$expected" = success ]; then
        [ "$status" = 0 ] || { printf 'FAIL %s (see %s)\n' "$name" "$TEST_ROOT/log"; exit 1; }
        [ -x "${ROCKUP_HOME-$HOME/.rockup}/bin/rockup" ]
        [ "$(wc -l < "$TEST_ROOT/urls")" -eq 2 ]
        [ "$(cat "$TEST_ROOT/invocation")" = "$(printf 'self install\ninstall %s' "$EXPECTED_CHANNEL")" ]
        grep -F 'Rock installed.' "$TEST_ROOT/log" >/dev/null
    else
        [ "$status" != 0 ] || { printf 'FAIL %s unexpectedly succeeded\n' "$name"; exit 1; }
        case "$TEST_MODE" in
            install-failure) [ "$(cat "$TEST_ROOT/invocation")" = 'self install' ] ;;
            toolchain-failure)
                [ "$(cat "$TEST_ROOT/invocation")" = "$(printf 'self install\ninstall %s' "$EXPECTED_CHANNEL")" ]
                [ -x "$ROCKUP_HOME/bin/rockup" ]
                grep -F 'toolchain installation failed' "$TEST_ROOT/log" >/dev/null
                ! grep -F 'Rock installed.' "$TEST_ROOT/log" >/dev/null
                ;;
            *) [ ! -e "$TEST_ROOT/invocation" ] ;;
        esac
    fi
    [ -z "$(find "$TMPDIR" -mindepth 1 -print)" ]
    case "$name" in
        existing) cmp "$FIXTURES/rockup" "$ROCKUP_HOME/bin/rockup" ;;
        dangling) [ -L "$ROCKUP_HOME/bin/rockup" ] ;;
    esac
    printf 'PASS %s\n' "$name"
}

TEST_MODE=valid
EXPECTED_CHANNEL=stable
EXPECTED_RELEASE_PATH=latest/download
export TEST_MODE EXPECTED_CHANNEL EXPECTED_RELEASE_PATH
run_case stable success
run_case default-home success
EXPECTED_RELEASE_PATH=download/v0.1.0
EXPECTED_CHANNEL=v0.1.0
run_case pinned success v0.1.0
EXPECTED_RELEASE_PATH=latest/download
EXPECTED_CHANNEL=stable
for TEST_MODE in download-failure missing-sidecar mismatch wrong-name traversal extra-line install-failure toolchain-failure; do
    run_case "$TEST_MODE" failure
done
TEST_MODE=valid
for name in existing dangling empty-home relative-home; do
    run_case "$name" failure
    [ ! -e "$TEST_ROOT/urls" ]
done
run_case invalid-version failure v../escape
run_case bare-version failure 0.1.0
run_case extra-argument failure stable extra
# macOS hosts select their own manager asset.
TEST_OS=Darwin TEST_ARCH=arm64 EXPECTED_TARGET=aarch64-apple-darwin
export TEST_OS TEST_ARCH EXPECTED_TARGET
run_case darwin-arm64 success
TEST_ARCH=x86_64 EXPECTED_TARGET=x86_64-apple-darwin
run_case darwin-x86_64 success
unset TEST_OS TEST_ARCH EXPECTED_TARGET
TEST_OS=FreeBSD
export TEST_OS
run_case unsupported-os failure
unset TEST_OS
TEST_ARCH=aarch64
export TEST_ARCH
run_case unsupported-arch failure
unset TEST_ARCH
TEST_LIBC='glibc 2.38'
export TEST_LIBC
run_case old-glibc failure
TEST_LIBC=musl
run_case musl failure
unset TEST_LIBC
# rockup unpacks toolchains itself: the system tar does not matter.
TEST_TAR=bsdtar
export TEST_TAR
run_case non-gnu-tar success
