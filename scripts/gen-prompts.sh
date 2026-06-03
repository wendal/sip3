#!/usr/bin/env bash
# scripts/gen-prompts.sh - Generate system prompts for the voicemail IVR
#
# Outputs 8kHz mono PCM16 WAV files into voicemail/prompts/<lang>/
# Languages: en (default), zh (Mandarin via espeak-ng cmn voice)
#
# Usage:
#   ./scripts/gen-prompts.sh           # generate both en and zh
#   ./scripts/gen-prompts.sh en        # english only
#   ./scripts/gen-prompts.sh zh        # chinese only
#
# Requires: espeak-ng (apt install espeak-ng, apk add espeak-ng,
#                        brew install espeak). On macOS, the default
#                        espeak does not include cmn voices — install
#                        the --with-voices variant or use `say` shim.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/voicemail/prompts"
mkdir -p "${OUT_DIR}/en" "${OUT_DIR}/zh"

gen_en() {
  local name="$1" text="$2"
  espeak-ng -v en-us -s 145 -p 50 -w "${OUT_DIR}/en/${name}.wav" "${text}"
}

gen_zh() {
  local name="$1" text="$2"
  # espeak-ng < 1.52 needs the explicit voice file path; new versions
  # accept the bare voice name `cmn`. We pass both forms for safety.
  espeak-ng -v cmn -s 140 -p 50 -w "${OUT_DIR}/zh/${name}.wav" "${text}" \
    || espeak-ng -v zh -s 140 -p 50 -w "${OUT_DIR}/zh/${name}.wav" "${text}"
}

# Phrase catalog. Keep these in sync with backend/src/sip/voicemail.rs
# prompt_key() calls and the *97 IVR state machine.
declare -A EN_PROMPTS=(
  ["pin_prompt"]="Please enter your PIN, followed by the pound key."
  ["pin_invalid"]="Incorrect PIN. Please try again."
  ["recording_stopped"]="Recording stopped."
  ["nav_previous"]="Previous message."
  ["nav_next"]="Next message."
  ["nav_deleted"]="Message deleted."
  ["nav_saved"]="Message saved."
  ["mailbox_full"]="Your mailbox is full. Please delete some messages."
)

declare -A ZH_PROMPTS=(
  ["pin_prompt"]="请输入您的密码,按井号键结束。"
  ["pin_invalid"]="密码错误,请重新输入。"
  ["recording_stopped"]="录音已结束。"
  ["nav_previous"]="上一条留言。"
  ["nav_next"]="下一条留言。"
  ["nav_deleted"]="留言已删除。"
  ["nav_saved"]="留言已保存。"
  ["mailbox_full"]="您的语音信箱已满,请删除一些留言。"
)

run_lang() {
  local lang="$1"
  echo "==> Generating ${lang} prompts into ${OUT_DIR}/${lang}/"
  case "${lang}" in
    en)
      for k in "${!EN_PROMPTS[@]}"; do gen_en "$k" "${EN_PROMPTS[$k]}"; done
      ;;
    zh)
      for k in "${!ZH_PROMPTS[@]}"; do gen_zh "$k" "${ZH_PROMPTS[$k]}"; done
      ;;
    *)
      echo "Unknown language: ${lang}" >&2; exit 1
      ;;
  esac
}

if [ "$#" -eq 0 ]; then
  run_lang en
  run_lang zh
else
  for lang in "$@"; do run_lang "${lang}"; done
fi

echo "Done. Set SIP3__SERVER__VOICEMAIL_PROMPT_LANG to 'en' or 'zh' to switch."
