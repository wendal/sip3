# Voicemail system prompts

The SIP3 voicemail IVR plays short WAV prompts at well-known points (PIN entry,
IVR nav feedback, mailbox-full warning, etc.). These prompts are loaded from
`<voicemail_prompt_dir>/<lang>/<name>.wav` where `<lang>` comes from
`SIP3__SERVER__VOICEMAIL_PROMPT_LANG` (default `en`).

**Operator-shipped content** — these files are not committed to the repo
because they're generated artifacts. Run the generator at deploy time:

```bash
./scripts/gen-prompts.sh           # en + zh
./scripts/gen-prompts.sh en        # english only
```

The generator requires `espeak-ng` (`apt install espeak-ng`,
`apk add espeak-ng`, `brew install espeak`). For Chinese it uses the
`cmn`/`zh` voice bundled with espeak-ng.

## Phrase catalog

The Rust side references these names in
`backend/src/sip/voicemail.rs::prompt_key()` (added in v1.9):

| name | en | zh |
|------|----|----|
| `pin_prompt` | "Please enter your PIN, followed by the pound key." | "请输入您的密码,按井号键结束。" |
| `pin_invalid` | "Incorrect PIN. Please try again." | "密码错误,请重新输入。" |
| `recording_stopped` | "Recording stopped." | "录音已结束。" |
| `nav_previous` | "Previous message." | "上一条留言。" |
| `nav_next` | "Next message." | "下一条留言。" |
| `nav_deleted` | "Message deleted." | "留言已删除。" |
| `nav_saved` | "Message saved." | "留言已保存。" |
| `mailbox_full` | "Your mailbox is full. Please delete some messages." | "您的语音信箱已满,请删除一些留言。" |

## Per-mailbox override

A mailbox owner can upload a custom greeting via the admin UI
(`POST /api/voicemail/boxes/:id/greeting`). That override always takes
precedence over the matching system prompt.

## Audio format

- Mono, 8 kHz sample rate, 16-bit PCM
- File extension must be `.wav`
- Maximum 60 seconds for a custom greeting (system prompts are typically <5s)
- Validated by `read_pcm16_wav` in `backend/src/storage/voicemail.rs` on upload
