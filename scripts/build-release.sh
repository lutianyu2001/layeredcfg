#!/usr/bin/env bash
# Builds the files of a release for PyPI, the source distribution and the wheels, from a
# clean checkout of its tag into dist/, and checks that none of them names a path of this
# machine. maturin writes into each wheel a CycloneDX SBOM (.dist-info/sboms/) that holds
# the absolute path of the checkout, which no option rewrites: so the checkout is made in
# a private directory of XDG_RUNTIME_DIR (/run/user/<uid>), whose path names no one. rustc
# writes the paths of the sources into the panic messages: those under the home directory
# and the cargo home are remapped (Cargo's trim-paths is not stable yet). The wheels hold the
# crates compiled into the extension, whose license texts must ship with them: they are
# collected from the tag's Cargo.lock, and every file is checked to hold them.
#
# Usage: scripts/build-release.sh <tag>
# Runs the same way on a machine of one's own and in the release workflow.
# Needs maturin (MATURIN, or on PATH), zig for the targets of other platforms, the targets
# installed with rustup, uvx for twine, and cargo-about at the version that the project's
# scripts/third-party-licenses.sh names.
# Only the targets and the Python version below differ between projects (Per project).
set -euo pipefail

tag="${1:?usage: scripts/build-release.sh <tag>}"
root="$(git rev-parse --show-toplevel)"
cd "$root"
git rev-parse -q --verify "refs/tags/$tag^{commit}" >/dev/null || {
    echo "no tag $tag" >&2
    exit 1
}

# Per project: each target triple and the platform its wheel claims. abi3 wheels are built
# once, for the oldest Python the package supports.
targets=(
    "x86_64-unknown-linux-gnu manylinux2014"
    "aarch64-unknown-linux-gnu manylinux2014"
    "x86_64-unknown-linux-musl musllinux_1_2"
    "aarch64-unknown-linux-musl musllinux_1_2"
)
python="python3.10"
maturin="${MATURIN:-maturin}"

# In GitHub Actions the runner is a machine of its own, so /tmp serves when XDG_RUNTIME_DIR is not
# set. Not RUNNER_TEMP: it is under the home directory, which the check below looks for.
base="${XDG_RUNTIME_DIR:-}"
[ -n "$base" ] || [ "${GITHUB_ACTIONS:-}" != true ] || base=/tmp
[ -n "$base" ] || {
    echo "XDG_RUNTIME_DIR is not set: no private directory whose path names no one" >&2
    exit 1
}
work="$(mktemp -d "$base/release.XXXXXXXX")"
src="$work/src"
out="$work/dist"
cleanup() {
    git -C "$root" worktree remove --force "$src" 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT
git worktree add -q --detach "$src" "$tag"

# Into the checkout's third-party/, which pyproject.toml's license-files names: maturin then
# puts it into the sdist and into each wheel's .dist-info/licenses/. maturin 1.15.0 leaves
# out a pattern that matches nothing without a word, hence the check below.
licenses="third-party/rust-licenses.html"
collect="$src/scripts/third-party-licenses.sh"
[ -x "$collect" ] || {
    echo "$tag has no scripts/third-party-licenses.sh to collect the license texts" >&2
    exit 1
}
"$collect" "$src/third-party"

# Kept between runs, outside the checkout (a few GB, more than XDG_RUNTIME_DIR holds).
export CARGO_TARGET_DIR="$root/target/release-files"
# Separated by 0x1f, so that a path with a space stays one argument. The last prefix that
# matches is the one applied.
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
sep=$'\x1f'
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$HOME=~$sep--remap-path-prefix=$cargo_home=/cargo$sep--remap-path-prefix=$src=."

cd "$src"
"$maturin" sdist -o "$out"
for entry in "${targets[@]}"; do
    read -r target compatibility <<<"$entry"
    "$maturin" build --release --locked --zig --target "$target" --compatibility "$compatibility" \
        -i "$python" -o "$out"
done
cd "$root"

# What must not be in any file: the home directory (with this checkout, the cargo home and
# the toolchains under it), and a TMPDIR other than /tmp.
leaks=("$HOME" "$root")
case "${TMPDIR:-/tmp}" in /tmp | /tmp/) ;; *) leaks+=("$TMPDIR") ;; esac
patterns=()
for leak in "${leaks[@]}"; do patterns+=(-e "$leak"); done
failed=0
for file in "$out"/*; do
    unpacked="$work/unpacked/$(basename "$file")"
    mkdir -p "$unpacked"
    case "$file" in
        *.whl) unzip -q "$file" -d "$unpacked" ;;
        *.tar.gz) tar -xzf "$file" -C "$unpacked" ;;
        *) echo "$file: not a wheel or a source distribution" >&2; failed=1; continue ;;
    esac
    case "$file" in
        *.whl) held=("$unpacked"/*.dist-info/licenses/"$licenses") ;;
        *) held=("$unpacked"/*/"$licenses") ;;
    esac
    [ -s "${held[0]}" ] || {
        echo "$(basename "$file") does not hold $licenses (license-files in pyproject.toml)" >&2
        failed=1
    }
    found="$(grep -rlaF "${patterns[@]}" "$unpacked")" && status=0 || status=$?
    case "$status" in
        0)
            echo "$(basename "$file") names a path of this machine in:" >&2
            sed "s|^$unpacked/|  |" <<<"$found" >&2
            failed=1
            ;;
        1) ;;
        *) echo "$(basename "$file"): grep failed" >&2; failed=1 ;;
    esac
done
[ "$failed" = 0 ] || exit 1
uvx twine==7.0.0 check --strict "$out"/*

rm -rf "$root/dist"
mkdir "$root/dist"
cp "$out"/* "$root/dist/"
(cd "$root/dist" && sha256sum -- *)
