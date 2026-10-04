#!/usr/bin/env bash
#
# agile-md installer — install the `amd` command onto your PATH.
#
#   ./install.sh [--dir DIR] [--version vX.Y.Z] [--from-source]
#   curl -fsSL https://raw.githubusercontent.com/mrdoodles/agile-md/v4/install.sh | bash
#
# Default target directory: /usr/local/bin if writable, else ~/bin.
# Run from a clone with cargo installed and it builds from source; otherwise it
# downloads the prebuilt binary for your platform from the GitHub release.
#
# Installs two commands: `amd`, the command line interface, and `amdm`, the management interface
# board. Both come from the release zip and from a source build.
#
# Then, in any project directory:  amd init project-name will initialise your tasks board"
#
set -euo pipefail

REPO="mrdoodles/agile-md"
DIR=""
VERSION="latest"
FROM_SOURCE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --dir) DIR="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --from-source) FROM_SOURCE=1; shift ;;
    *) shift ;;
  esac
done

if [ -z "${DIR}" ]; then
  if [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
    DIR="/usr/local/bin"
  else
    DIR="${HOME}/bin"
  fi
fi

SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

die() { printf 'install: %s\n' "$*" >&2; exit 1; }

TMP=""
cleanup() { if [ -n "${TMP}" ]; then rm -rf "${TMP}"; fi; }
trap cleanup EXIT

# Rust target triple for this machine — matches the assets published by
# mrdoodles/rust-release (amd-<target>.zip).
target_triple() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "${arch}" in
    arm64|aarch64) arch="aarch64" ;;
    x86_64|amd64) arch="x86_64" ;;
    *) die "unsupported architecture '${arch}' — install with --from-source" ;;
  esac
  case "${os}" in
    Linux) printf '%s-unknown-linux-gnu' "${arch}" ;;
    Darwin) printf '%s-apple-darwin' "${arch}" ;;
    *) die "unsupported OS '${os}' — install with --from-source" ;;
  esac
}

build_from_source() {
  command -v cargo >/dev/null 2>&1 || die "cargo not found — install Rust from https://rustup.rs"
  printf 'Building amd from source...\n'
  ( cd "${SELF}" && cargo build --release --locked )
  mkdir -p "${DIR}"
  cp "${SELF}/target/release/amd" "${DIR}/amd"
  # The desktop board is a default feature, but --no-default-features is a
  # supported way to build, and then there is no amdui to copy.
  if [ -f "${SELF}/target/release/amdui" ]; then
    cp "${SELF}/target/release/amdui" "${DIR}/amdui"
  fi
}

download_release() {
  command -v curl >/dev/null 2>&1 || die "curl not found"
  command -v unzip >/dev/null 2>&1 || die "unzip not found — install it, or use --from-source"

  local triple url
  triple="$(target_triple)"
  if [ "${VERSION}" = "latest" ]; then
    url="https://github.com/${REPO}/releases/latest/download/amd-${triple}.zip"
  else
    url="https://github.com/${REPO}/releases/download/${VERSION}/amd-${triple}.zip"
  fi

  TMP="$(mktemp -d)"
  printf 'Downloading %s\n' "${url}"
  curl -fsSL "${url}" -o "${TMP}/amd.zip" || return 1
  unzip -q -o "${TMP}/amd.zip" -d "${TMP}" || return 1
  [ -f "${TMP}/amd" ] || return 1
  mkdir -p "${DIR}"
  cp "${TMP}/amd" "${DIR}/amd"
  # Guarded rather than assumed: `--version` can pin a release made before the
  # zip carried both binaries, and `amd gui` is the way in to the board there.
  if [ -f "${TMP}/amdui" ]; then
    cp "${TMP}/amdui" "${DIR}/amdui"
  fi
}

if [ "${FROM_SOURCE}" = "1" ]; then
  build_from_source
elif [ -f "${SELF}/Cargo.toml" ] && command -v cargo >/dev/null 2>&1; then
  build_from_source
elif ! download_release; then
  printf 'No prebuilt binary available; falling back to a source build.\n' >&2
  build_from_source
fi

chmod +x "${DIR}/amd"

echo "Installed amd -> ${DIR}/amd"
"${DIR}/amd" --version || true
if [ -f "${DIR}/amdui" ]; then
  chmod +x "${DIR}/amdui"
  echo "Installed amdui -> ${DIR}/amdui   (the desktop board)"
fi
case ":${PATH}:" in
  *":${DIR}:"*) ;;
  *) echo "Note: ${DIR} is not on your PATH. Add this to your shell profile:"
     echo "      export PATH=\"${DIR}:\$PATH\"" ;;
esac
echo "Then, in any repo:  amd init  &&  amd new \"My first task\""
echo "For the desktop board:  amdui   (or amd gui)"
