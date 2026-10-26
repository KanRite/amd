#!/usr/bin/env bash
# Installs Ubuntu packages on a CI runner without hanging (ticket 327).
#
#   scripts/apt-install.sh PACKAGE...
#
# Ubuntu runners have hung in apt-get for ten minutes and more with nothing
# printed. So apt retries and gives up on a server that stops answering,
# waits a while for another apt to let go of its lock instead of failing at
# once, each `update` is cut off after three minutes and tried again (up to
# three times), and `update` prints each source it fetches, so a hang shows
# where it is.
set -euo pipefail

opts=(
  -o Acquire::Retries=3
  -o Acquire::http::Timeout=30
  -o Acquire::https::Timeout=30
  -o DPkg::Lock::Timeout=120
)

for attempt in 1 2 3; do
  if sudo DEBIAN_FRONTEND=noninteractive timeout 180 apt-get "${opts[@]}" update; then
    break
  fi
  if [ "${attempt}" -eq 3 ]; then
    echo "::error::apt-get update failed three times"
    exit 1
  fi
  echo "::warning::apt-get update failed or took over 3 minutes (attempt ${attempt}); trying again"
done

sudo DEBIAN_FRONTEND=noninteractive timeout 600 apt-get "${opts[@]}" install -y -q "$@"
