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
  "$SKOPEO_BIN" inspect "docker://${DST_PREFIX}/${name}:${TAG}"
}

copy_image backend
copy_image frontend
inspect_image backend
inspect_image frontend

printf 'Synced backend and frontend for tag %s\n' "$TAG"
