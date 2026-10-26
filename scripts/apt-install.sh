#!/usr/bin/env bash
# Installs Ubuntu packages on a CI runner without hanging (ticket 327).
#
#   scripts/apt-install.sh PACKAGE...
#
# Ubuntu runners have sat in apt-get for ten minutes and more. The runners'
# own mirror (azure.archive.ubuntu.com) has been seen answering at about
# 40 KB/s, which apt's timeouts don't catch: they only notice a server that
# goes silent. So each apt-get is cut off after a few minutes, `update` is
# tried again, and an install that doesn't finish moves to Ubuntu's main
# archive and carries on (what was downloaded is kept). apt also waits a
# while for another apt's lock instead of failing at once, and prints each
# file it fetches, so a slow one shows.
set -euo pipefail

opts=(
  -o Acquire::Retries=3
  -o Acquire::http::Timeout=30
  -o Acquire::https::Timeout=30
  -o DPkg::Lock::Timeout=120
)

apt() {
  local limit="$1"
  shift
  sudo DEBIAN_FRONTEND=noninteractive timeout "${limit}" apt-get "${opts[@]}" "$@"
}

update() {
  local attempt
  for attempt in 1 2 3; do
    if apt 120 update; then
      return 0
    fi
    echo "::warning::apt-get update failed or took over 2 minutes (attempt ${attempt})"
  done
  echo "::error::apt-get update failed three times"
  return 1
}

# The runners read their mirrors from this list (apt's mirror+file method),
# the Azure one first. Ubuntu's main archive goes first instead.
use_main_archive() {
  local list=/etc/apt/apt-mirrors.txt
  if [ ! -f "${list}" ]; then
    return 1
  fi
  printf '%s\tpriority:1\n%s\tpriority:2\n' \
    http://archive.ubuntu.com/ubuntu/ http://azure.archive.ubuntu.com/ubuntu/ |
    sudo tee "${list}" >/dev/null
}

update
if apt 300 install -y -q "$@"; then
  exit 0
fi
echo "::warning::apt-get install failed or took over 5 minutes; trying Ubuntu's main archive"
use_main_archive || echo "::warning::no /etc/apt/apt-mirrors.txt; trying the same mirrors again"
update
apt 600 install -y -q "$@"
