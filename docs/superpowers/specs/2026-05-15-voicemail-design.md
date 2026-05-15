# SIP3 Voicemail Design

## Goal

Add a Linphone-compatible voicemail feature to SIP3 after the conference feature. The MVP must provide a complete basic voicemail loop: offline and no-answer calls can be recorded, users can dial `*97` to hear their own mailbox, and Linphone can receive MWI updates through standard message-summary subscriptions.

## Confirmed decisions

- MVP scope: offline/no-answer voicemail, recording, phone playback, and MWI.
- Voicemail access: users dial `*97`; SIP3 identifies the mailbox from the caller's `From` extension. No PIN in MVP.
- Audio storage: design a storage abstraction for future object storage, but MVP stores local WAV files and MySQL metadata.
- No-answer timeout: default 25 seconds, configurable.
- MWI: standard `SUBSCRIBE`/`NOTIFY` with `Event: message-summary`; no unsolicited MWI in MVP.
- Playback menu: `1` replay, `2` or `#` next, `7` delete, `9` save, `*` exit.
- Prompts: deployable/replaceable WAV prompt files; no TTS dependency.

## Architecture

Voicemail is implemented as a local SIP B2BUA endpoint, not as a pure proxy rule. It reuses SIP parsing and response helpers from `sip::handler`, G.711 helpers from the conference work, and the existing registration/account database model.

There are two local voicemail entry points:

1. Call delivery to a mailbox when a user is offline or does not answer.
2. Mailbox access when a user dials `*97`.

Ordinary user-to-user calls continue to use the existing proxy path. The proxy only hands a call to voicemail when the target mailbox is enabled and the delivery condition is met.

## Call delivery flow

1. Caller sends `INVITE` to a normal account extension.
2. If the callee is unregistered and has voicemail enabled, SIP3 answers the caller locally with voicemail SDP and starts recording.
3. If the callee is registered, SIP3 forwards the `INVITE` normally and starts a configurable no-answer timer.
4. If a successful callee answer arrives before the timer fires, SIP3 cancels the timer and the call remains a normal proxied call.
5. If the timer fires first, SIP3 sends `CANCEL` to the callee and answers the caller locally with voicemail SDP.
6. Recording ends when the caller sends `BYE`, presses `#`, reaches the max message length, or media becomes inactive for the configured idle timeout.
7. SIP3 finalizes the WAV file, inserts the voicemail message row, and sends MWI updates to active subscriptions.

Busy handling is deferred. If the callee rejects the call with busy-style final responses in MVP, SIP3 keeps the existing response behavior unless a later plan explicitly adds busy-to-voicemail.

## Mailbox access flow

1. User dials `*97`.
2. SIP3 verifies that the caller extension exists and has an enabled mailbox.
3. SIP3 answers with local RTP/AVP G.711 SDP.
4. SIP3 plays mailbox prompts and voicemail WAV files to the caller.
5. DTMF controls playback:
   - `1`: replay current message.
   - `2` or `#`: next message.
   - `7`: mark current message deleted.
   - `9`: mark current message saved.
   - `*`: exit.
6. Read/delete/save actions update message status and trigger MWI NOTIFY when unread counts change.

## Media design

Voicemail supports plain RTP/AVP G.711 PCMU and PCMA for the MVP. SRTP/SAVP, Opus, video, and browser WebRTC voicemail are out of scope.

Recording sessions:

- allocate one UDP socket from a dedicated voicemail RTP range;
- learn the caller RTP peer from the first inbound packet;
- decode G.711 to PCM16 at 8 kHz;
- stream samples into a WAV writer;
- handle RFC 2833 telephone-event and SIP `INFO application/dtmf-relay` for `#` end-recording;
- enforce max duration and idle timeout.

Playback sessions:

- read WAV PCM16/8 kHz files;
- encode to the caller's negotiated PCMU or PCMA payload type;
- send RTP with generated sequence numbers, timestamps, and SSRC;
- accept RFC 2833 and SIP INFO DTMF for menu navigation.

Prompt WAV files are required deployment assets. Missing or invalid prompts are treated as configuration errors and are logged explicitly instead of silently continuing.

## Data model

Add `sip_voicemail_boxes`:

- `id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY`
- `username VARCHAR(64) NOT NULL`
- `domain VARCHAR(128) NOT NULL`
- `enabled TINYINT(1) NOT NULL DEFAULT 1`
- `no_answer_secs INT UNSIGNED NOT NULL DEFAULT 25`
- `max_message_secs INT UNSIGNED NOT NULL DEFAULT 120`
- `max_messages INT UNSIGNED NOT NULL DEFAULT 100`
- `greeting_storage_key VARCHAR(512) NULL`
- `created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP`
- `updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP`
- `UNIQUE KEY uniq_voicemail_box (username, domain)`
- foreign key `(username, domain)` references `sip_accounts(username, domain)` and cascades delete/update with the account.

Add `sip_voicemail_messages`:

- `id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY`
- `box_id BIGINT UNSIGNED NOT NULL`
- `caller VARCHAR(128) NOT NULL`
- `callee VARCHAR(128) NOT NULL`
- `call_id VARCHAR(255) NOT NULL`
- `duration_secs INT UNSIGNED NOT NULL DEFAULT 0`
- `storage_key VARCHAR(512) NOT NULL`
- `content_type VARCHAR(128) NOT NULL DEFAULT 'audio/wav'`
- `status VARCHAR(32) NOT NULL DEFAULT 'new'`
- `created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP`
- `heard_at DATETIME NULL`
- indexes by mailbox/status/time and call ID.

Valid statuses are `new`, `saved`, and `deleted`. Deleted messages remain as metadata until a retention cleanup removes the file and row.

Add `sip_voicemail_mwi_subscriptions`:

- `id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY`
- `subscriber VARCHAR(64) NOT NULL`
- `domain VARCHAR(128) NOT NULL`
- `call_id VARCHAR(255) NOT NULL`
- `subscriber_tag VARCHAR(128) NOT NULL`
- `subscriber_ip VARCHAR(45) NOT NULL`
- `subscriber_port SMALLINT UNSIGNED NOT NULL`
- `expires_at DATETIME NOT NULL`
- `cseq INT UNSIGNED NOT NULL DEFAULT 1`
- unique key by subscriber/domain/call_id.

## Configuration

Add server configuration:

- `voicemail_access_extension = "*97"`
- `voicemail_no_answer_secs = 25`
- `voicemail_max_message_secs = 120`
- `voicemail_idle_timeout_secs = 10`
- `voicemail_storage_dir = "voicemail"`
- `voicemail_prompt_dir = "voicemail/prompts"`
- `voicemail_rtp_port_min = 10200`
- `voicemail_rtp_port_max = 10299`

The voicemail RTP range is separate from existing relay RTP (`10000-10099`), conference RTP (`10100-10199`), and WebRTC (`20000-20099`).

## Backend modules

- `backend/src/sip/voicemail.rs`: SIP endpoint, delivery/access session lifecycle, no-answer timer coordination, SIP response generation.
- `backend/src/sip/voicemail_media.rs`: RTP recording, WAV writing, WAV playback, DTMF parsing, media timeouts.
- `backend/src/sip/voicemail_mwi.rs`: message-summary subscriptions and NOTIFY generation.
- `backend/src/models/voicemail.rs`: typed rows and API DTOs.
- `backend/src/api/voicemail.rs`: admin APIs.
- `backend/src/storage/voicemail.rs`: `VoicemailStorage` trait and local filesystem implementation.
- `backend/src/storage/mod.rs`: export the voicemail storage module.

The implementation should avoid holding async mutex guards across awaits in RTP send/receive loops.

## API and UI

Add protected admin APIs:

- `GET /api/voicemail/boxes`
- `POST /api/voicemail/boxes`
- `PUT /api/voicemail/boxes/:id`
- `GET /api/voicemail/messages`
- `GET /api/voicemail/messages/:id/download`
- `PUT /api/voicemail/messages/:id`
- `DELETE /api/voicemail/messages/:id`

Add a Vue admin page named "语音信箱" with:

- mailbox list: account, enabled state, unread count, saved count, max message count, no-answer seconds;
- message list: caller, callee/mailbox, duration, status, created time;
- actions: play in browser, download WAV, mark read/saved, delete.

## MWI behavior

SIP3 handles `SUBSCRIBE` requests with `Event: message-summary` separately from the existing presence handler. Unsupported events still use current behavior.

On subscription, SIP3 stores or refreshes the subscription and immediately sends a NOTIFY with `application/simple-message-summary`, for example:

```text
Messages-Waiting: yes
Message-Account: sip:1001@example.com
Voice-Message: 2/1 (0/0)
```

The first number is unread/new messages. The second number is saved/old messages. On new voicemail, read, save, or delete actions, SIP3 sends NOTIFY to active subscriptions for the affected mailbox.

## Error handling

- Disabled mailbox: preserve existing call behavior; do not record.
- Unsupported offer: return `488 Not Acceptable Here`; do not create a message.
- Storage write failure: end the session with an error response if possible, log the error, remove partial files, and do not insert a successful message row.
- Prompt missing or unreadable: fail mailbox access explicitly and log the missing prompt path.
- Restart recovery: remove temporary files for recordings that were never finalized; only finalized recordings have database rows.
- Port exhaustion: return a clear SIP failure response and log the exhausted range.

## Tests

Backend tests should cover:

- mailbox extension routing for `*97`;
- offline delivery to voicemail without hijacking ordinary registered calls;
- no-answer timer state transitions;
- SDP negotiation for PCMU and PCMA;
- rejection of unsupported encrypted-only or non-G.711 offers;
- WAV writer finalizes a playable header and sample count;
- DTMF state for recording end and playback controls;
- MWI NOTIFY body formatting and unread/saved counts;
- API validation for mailbox settings and message status transitions.

Frontend build should verify the admin UI compiles. Manual Linphone smoke tests should cover offline voicemail, no-answer voicemail, `*97` playback, DTMF controls, and MWI icon/count updates.

## Out of scope

- PIN authentication for mailbox access.
- Busy-to-voicemail behavior.
- Email notifications.
- SRTP/SAVP voicemail media.
- Opus transcoding.
- Browser/WebRTC voicemail recording or playback over SIP.js.
- Object storage implementation beyond the storage abstraction.
