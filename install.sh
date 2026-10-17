#!/usr/bin/env bash
#
# agile-md installer — install the `amd` command onto your PATH.
#
#   ./install.sh [--dir DIR] [--version vX.Y.Z] [--from-source]
#   curl -fsSL https://raw.githubusercontent.com/agile-md/amd/v4/install.sh | bash
#
# Default target directory: /usr/local/bin if writable, else ~/bin.
# Run from a clone with cargo installed and it builds from source; otherwise it
# downloads the prebuilt binary for your platform from the GitHub release.
#
# Installs two commands: `amd`, the command line interface, and `amdui`, the
# desktop board. Both come from the release zip and from a source build; the
# board's source build needs Trunk and the wasm32-unknown-unknown target (and
# WebKitGTK on Linux), and is skipped with a note when they're missing.
#
# On Linux, installing amdui from a clone also installs its launcher entry and
# icon into ~/.local/share ($XDG_DATA_HOME), which is where the dock and top
# bar take the board's icon from.
#
# Then, in any project directory:  amd init project-name will initialise your tasks board"
#
set -euo pipefail

REPO="agile-md/amd"
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

# Puts SRC in place as DIR/NAME without writing into the installed file:
# Linux refuses to ("Text file busy") while that binary is running, which
# left an install half done (ticket 098). A copy beside it, then a rename,
# replaces the directory entry in one step; a running process keeps the old
# file until it exits, and the next launch gets the new one. Same directory,
# so the rename never crosses a filesystem.
replace_bin() {
  local src="$1" name="$2" dest="${DIR}/$2" new
  new="${DIR}/.${name}.new.$$"
  if [ -n "$(running "${dest}")" ]; then
    RESTART="${RESTART} ${name}"
  fi
  cp "${src}" "${new}" || { rm -f "${new}"; die "couldn't copy ${name} into ${DIR}"; }
  chmod +x "${new}"
  mv -f "${new}" "${dest}" || { rm -f "${new}"; die "couldn't replace ${dest}"; }
}

# The ids of processes running the binary at PATH, where that can be told
# (Linux's /proc). Empty elsewhere, and when nothing is.
running() {
  local exe pid target
  [ -d /proc/self ] && [ -e "$1" ] || return 0
  target="$(readlink -f "$1")"
  for exe in /proc/[0-9]*/exe; do
    [ "$(readlink "${exe}" 2>/dev/null)" = "${target}" ] || continue
    pid="${exe#/proc/}"
    printf '%s ' "${pid%/exe}"
  done
}

RESTART=""

# The desktop shows an app's icon in its dock and top bar from the app's
# launcher entry, matched by app id, not from the window. amdui's app id is
# `amdui` (Tauri leaves GTK's as the program name unless enableGTKAppId is
# set, which would also make every amdui window share one process), so the
# entry is amdui.desktop. The icons come from the source tree: a release
# download run outside a clone has none to install, and says so.
install_launcher() {
  local src data apps icons size
  [ "$(uname -s)" = "Linux" ] && [ -x "${DIR}/amdui" ] || return 0
  src="${SELF}/crates/amd-ui"
  if [ ! -f "${src}/app-icon.svg" ]; then
    printf 'No launcher for amdui: its icon is installed from a clone of the repository.\n' >&2
    return 0
  fi
  data="${XDG_DATA_HOME:-${HOME}/.local/share}"
  apps="${data}/applications"
  icons="${data}/icons/hicolor"
  mkdir -p "${apps}" "${icons}/scalable/apps"
  cp "${src}/app-icon.svg" "${icons}/scalable/apps/amdui.svg"
  # <size directory>:<generated file>, all made from app-icon.svg.
  for size in 32x32:32x32 128x128:128x128 256x256:128x128@2x; do
    mkdir -p "${icons}/${size%%:*}/apps"
    cp "${src}/icons/${size#*:}.png" "${icons}/${size%%:*}/apps/amdui.png"
  done
  cat > "${apps}/amdui.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=agile-md
GenericName=Kanban board
Comment=The agile-md desktop board
Exec="${DIR}/amdui"
Icon=amdui
Terminal=false
Categories=Development;ProjectManagement;
StartupWMClass=amdui
EOF
  # Caches some desktops read; harmless where they're not used or not there.
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t "${icons}" 2>/dev/null || true
  fi
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "${apps}" 2>/dev/null || true
  fi
  echo "Installed the amdui launcher -> ${apps}/amdui.desktop"
}

# Rust target triple for this machine — matches the assets this repository's
# release workflow publishes (amd-<target>.zip).
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

# Whether this machine can build the desktop board: its page is compiled to
# WebAssembly by Trunk before amdui itself, which embeds it.
can_build_board() {
  command -v trunk >/dev/null 2>&1 \
    && rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'
}

build_from_source() {
  command -v cargo >/dev/null 2>&1 || die "cargo not found — install Rust from https://rustup.rs"
  printf 'Building amd from source...\n'
  ( cd "${SELF}" && cargo build --release --locked -p amd-cli )
  mkdir -p "${DIR}"
  replace_bin "${SELF}/target/release/amd" amd
  # The desktop board is optional: without its build tools, or without
  # WebKitGTK on Linux, amd is still installed and this says what's missing.
  if ! can_build_board; then
    printf 'Skipping the desktop board (amdui): it needs Trunk and the wasm target —\n' >&2
    printf '  cargo install --locked trunk && rustup target add wasm32-unknown-unknown\n' >&2
    printf 'and on Linux WebKitGTK and ALSA (libwebkit2gtk-4.1-dev libasound2-dev). Then run install.sh again.\n' >&2
    return 0
  fi
  printf 'Building the desktop board (amdui)...\n'
  if ( cd "${SELF}/crates/amd-ui-web" && trunk build --release ) \
    && ( cd "${SELF}" && cargo build --release --locked -p amd-ui ); then
    replace_bin "${SELF}/target/release/amdui" amdui
  else
    printf 'The desktop board did not build (on Linux it needs libwebkit2gtk-4.1-dev and libasound2-dev);\n' >&2
    printf 'amd is installed without it.\n' >&2
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
  replace_bin "${TMP}/amd" amd
  # Guarded rather than assumed: `--version` can pin a release made before the
  # zip carried both binaries, and `amd gui` is the way in to the board there.
  if [ -f "${TMP}/amdui" ]; then
    replace_bin "${TMP}/amdui" amdui
    check_board_libraries
  fi
}

# A downloaded amdui is linked against WebKitGTK on Linux (ticket 085). It
# installs either way; this says what to add when the libraries are missing,
# rather than leaving a board that won't start with no reason given.
check_board_libraries() {
  [ "$(uname -s)" = "Linux" ] || return 0
  command -v ldd >/dev/null 2>&1 || return 0
  local missing
  missing="$(ldd "${DIR}/amdui" 2>/dev/null | awk '/not found/ {print $1}' | tr '\n' ' ')"
  [ -n "${missing}" ] || return 0
  printf 'Note: the desktop board (amdui) needs libraries this machine lacks:\n  %s\n' "${missing}" >&2
  printf 'Install WebKitGTK 4.1 and ALSA, e.g. Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-0 libasound2t64\n' >&2
  printf '  Fedora: sudo dnf install webkit2gtk4.1 alsa-lib    Arch: sudo pacman -S webkit2gtk-4.1 alsa-lib\n' >&2
  printf 'amd itself works without them.\n' >&2
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
  install_launcher
fi
case ":${PATH}:" in
  *":${DIR}:"*) ;;
  *) echo "Note: ${DIR} is not on your PATH. Add this to your shell profile:"
     echo "      export PATH=\"${DIR}:\$PATH\"" ;;
esac
echo "Then, in any repo:  amd init  &&  amd new \"My first task\""
if [ -x "${DIR}/amdui" ]; then
  echo "For the desktop board:  amdui   (or amd gui)"
fi
for name in ${RESTART}; do
  echo "Note: ${name} was running while it was replaced; restart it to use the new build."
done
