#!/usr/bin/env bash
set -euo pipefail

TAG="${1:?usage: sync-from-ghcr.sh <tag>}"
SKOPEO_BIN="${SKOPEO_BIN:-skopeo}"
SRC_PREFIX="${SRC_PREFIX:-ghcr.io/wendal/sip3}"
DST_PREFIX="${DST_PREFIX:-harbor.air32.cn/sip3}"

copy_image() {
  local name="$1"
  "$SKOPEO_BIN" copy --all \
    "docker://${SRC_PREFIX}/${name}:${TAG}" \
    "docker://${DST_PREFIX}/${name}:${TAG}"
}

inspect_image() {
  local name="$1"
  local inspect_cmd=(inspect --raw "docker://${DST_PREFIX}/${name}:${TAG}")

  if [[ "$SKOPEO_BIN" == echo ]]; then
    "$SKOPEO_BIN" "${inspect_cmd[@]}"
  else
    "$SKOPEO_BIN" "${inspect_cmd[@]}" >/dev/null
  fi
}

copy_image backend
copy_image frontend
inspect_image backend
inspect_image frontend

printf 'Synced backend and frontend for tag %s\n' "$TAG"
