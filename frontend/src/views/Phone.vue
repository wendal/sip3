<template>
  <div class="phone-page">
    <!-- Login / Settings Panel -->
    <div v-if="!ua" class="panel login-panel">
      <div class="phone-logo">📞 SIP3 软电话</div>
      <el-form :model="form" label-width="90px" @submit.prevent="connect">
        <el-form-item label="用户名">
          <el-input v-model="form.username" placeholder="sip 用户名" autocomplete="username" />
        </el-form-item>
        <el-form-item label="密码">
          <el-input v-model="form.password" type="password" placeholder="sip 密码" autocomplete="current-password" />
        </el-form-item>
        <el-form-item label="域名">
          <el-input v-model="form.domain" placeholder="sip.example.com" />
        </el-form-item>
        <el-form-item label="WSS 服务器">
          <el-input v-model="form.wssServer" placeholder="wss://sip.example.com:5443" />
        </el-form-item>
        <el-form-item>
          <el-button type="primary" native-type="submit" :loading="connecting" style="width:100%">
            连接注册
          </el-button>
        </el-form-item>
        <div v-if="statusMsg" class="status-msg error">{{ statusMsg }}</div>
      </el-form>
    </div>

    <!-- Connected: Softphone UI -->
    <div v-else class="panel softphone-panel">
      <!-- Header -->
      <div class="phone-header">
        <span class="phone-logo">📞 SIP3 软电话</span>
        <div class="reg-status">
          <span :class="['reg-dot', registered ? 'dot-green' : 'dot-yellow']"></span>
          <span>{{ registered ? '已注册' : '注册中…' }}</span>
          <span class="user-label">{{ form.username }}@{{ form.domain }}</span>
        </div>
        <el-button size="small" plain @click="disconnect">断开</el-button>
      </div>

      <!-- Number Display -->
      <div class="display-area">
        <div class="display-number">{{ displayNumber || (callState !== 'idle' ? callLabel : '\u00a0') }}</div>
        <div v-if="callState !== 'idle'" class="call-state-label">
          {{ callStateLabel }}
          <span v-if="callState === 'active'" class="timer">{{ formatDuration(callDuration) }}</span>
        </div>
        <div v-if="callError" class="call-error">{{ callError }}</div>
      </div>

      <!-- Incoming Call -->
      <div v-if="callState === 'incoming'" class="incoming-panel">
        <div class="incoming-label">📲 来电：{{ callLabel }}</div>
        <div class="btn-row">
          <el-button type="success" circle size="large" @click="answerCall">
            <el-icon><Phone /></el-icon>
          </el-button>
          <el-button type="danger" circle size="large" @click="rejectCall">
            <el-icon><CloseBold /></el-icon>
          </el-button>
        </div>
      </div>

      <!-- Dial Pad (idle or dialing) -->
      <div v-if="callState === 'idle' || callState === 'dialing'" class="dialpad">
        <div v-for="row in DIALPAD_KEYS" :key="row.map(key => key.digit).join('')" class="key-row">
          <button
            v-for="key in row"
            :key="key.digit"
            class="key-btn"
            @click="pressKey(key.digit)"
          >
            <span class="digit">{{ key.digit }}</span>
            <span class="sub">{{ key.sub }}</span>
          </button>
        </div>
        <div class="key-row action-row">
          <button class="key-btn key-call" :disabled="!displayNumber || callState === 'dialing'" @click="makeCall">
            <el-icon><Phone /></el-icon>
          </button>
          <button class="key-btn key-clear" @click="displayNumber = displayNumber.slice(0, -1)">⌫</button>
        </div>
      </div>

      <!-- In-Call Controls -->
      <div v-if="activeCallStates.includes(callState)" class="incall-controls">
        <el-button :type="muted ? 'warning' : ''" size="large" circle @click="toggleMute">
          <el-icon><Microphone /></el-icon>
        </el-button>
        <el-button type="danger" size="large" circle @click="hangup">
          <el-icon><CloseBold /></el-icon>
        </el-button>
      </div>

      <!-- Remote audio element -->
      <audio ref="remoteAudio" autoplay></audio>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onUnmounted } from 'vue'
import { activeCallStates, callFailureMessage } from '../utils/callUiState.mjs'
import { DIALPAD_KEYS } from '../utils/dialpad.mjs'
import {
  UserAgent,
  Registerer,
  RegistererState,
  Inviter,
  SessionState,
  Web,
} from 'sip.js'

// ── Form state ────────────────────────────────────────────────────────────────
const form = ref({
  username: '',
  password: '',
  domain: window.location.hostname,
  wssServer: `wss://${window.location.hostname}:5443`,
})

// ── SIP.js objects ────────────────────────────────────────────────────────────
const ua = ref(null)
const registerer = ref(null)
const currentSession = ref(null)

// ── UI state ──────────────────────────────────────────────────────────────────
const connecting = ref(false)
const registered = ref(false)
const statusMsg = ref('')
const displayNumber = ref('')
const callState = ref('idle') // idle | incoming | dialing | active
const callLabel = ref('')
const callError = ref('')
const muted = ref(false)
const callDuration = ref(0)
const remoteAudio = ref(null)

let callTimer = null

const callStateLabel = computed(() => ({
  incoming: '来电',
  dialing: '呼叫中…',
  active: '通话中',
}[callState.value] || ''))

// ── Helpers ───────────────────────────────────────────────────────────────────
function formatDuration(secs) {
  const m = Math.floor(secs / 60).toString().padStart(2, '0')
  const s = (secs % 60).toString().padStart(2, '0')
  return `${m}:${s}`
}

function pressKey(digit) {
  displayNumber.value += digit
}

function startCallTimer() {
  callDuration.value = 0
  callTimer = setInterval(() => callDuration.value++, 1000)
}

function stopCallTimer() {
  if (callTimer) { clearInterval(callTimer); callTimer = null }
  callDuration.value = 0
}

// Wire up a session's media and state transitions.
function attachSession(session, inbound) {
  currentSession.value = session

  session.stateChange.addListener((state) => {
    if (state === SessionState.Establishing) {
      callState.value = inbound ? 'incoming' : 'dialing'
    } else if (state === SessionState.Established) {
      callState.value = 'active'
      startCallTimer()
      // Route remote audio to the <audio> element.
      const sdh = session.sessionDescriptionHandler
      if (sdh && sdh.peerConnection) {
        const pc = sdh.peerConnection
        const remoteStream = new MediaStream()
        pc.getReceivers().forEach((rcv) => {
          if (rcv.track) remoteStream.addTrack(rcv.track)
        })
        if (remoteAudio.value) {
          remoteAudio.value.srcObject = remoteStream
        }
      }
    } else if (
      state === SessionState.Terminated ||
      state === SessionState.Terminating
    ) {
      stopCallTimer()
      callState.value = 'idle'
      callLabel.value = ''
      currentSession.value = null
      muted.value = false
      if (remoteAudio.value) remoteAudio.value.srcObject = null
    }
  })
}

// ── TURN credentials ──────────────────────────────────────────────────────────
async function fetchTurnCredentials() {
  try {
    const resp = await fetch('/api/turn/credentials', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        username: form.value.username,
        password: form.value.password,
      }),
    })
    if (!resp.ok) return []
    const data = await resp.json()
    return [
      {
        urls: data.uris,
        username: data.username,
        credential: data.password,
      },
    ]
  } catch {
    return []
  }
}

// ── Connect / disconnect ──────────────────────────────────────────────────────
async function connect() {
  statusMsg.value = ''
  if (!form.value.username || !form.value.password || !form.value.domain) {
    statusMsg.value = '请填写用户名、密码和域名'
    return
  }
  connecting.value = true
  try {
    const iceServers = await fetchTurnCredentials()

    const targetUri = UserAgent.makeURI(`sip:${form.value.username}@${form.value.domain}`)
    if (!targetUri) throw new Error('Invalid SIP URI')

    const userAgent = new UserAgent({
      uri: targetUri,
      transportOptions: { server: form.value.wssServer },
      authorizationUsername: form.value.username,
      authorizationPassword: form.value.password,
      sessionDescriptionHandlerFactoryOptions: {
        peerConnectionConfiguration: {
          iceServers: iceServers.length
            ? iceServers
            : [{ urls: `stun:${form.value.domain}:3478` }],
        },
      },
      // Handle incoming calls via delegate.
      delegate: {
        onInvite(invitation) {
          if (currentSession.value) {
            invitation.reject({ statusCode: 486 })
            return
          }
          callLabel.value = invitation.remoteIdentity?.uri?.user || '未知'
          callState.value = 'incoming'
          attachSession(invitation, true)
        },
      },
    })

    await userAgent.start()

    const reg = new Registerer(userAgent)
    reg.stateChange.addListener((state) => {
      registered.value = state === RegistererState.Registered
    })
    await reg.register()
    registered.value = reg.state === RegistererState.Registered

    ua.value = userAgent
    registerer.value = reg
  } catch (e) {
    statusMsg.value = `连接失败：${e.message}`
  } finally {
    connecting.value = false
  }
}

async function disconnect() {
  if (currentSession.value) {
    try { currentSession.value.bye() } catch {}
  }
  if (registerer.value) {
    try { await registerer.value.unregister() } catch {}
  }
  if (ua.value) {
    try { await ua.value.stop() } catch {}
  }
  ua.value = null
  registerer.value = null
  registered.value = false
  callState.value = 'idle'
  callLabel.value = ''
  stopCallTimer()
}

// ── Outbound call ─────────────────────────────────────────────────────────────
async function makeCall() {
  if (!ua.value || !displayNumber.value) return
  const target = UserAgent.makeURI(`sip:${displayNumber.value}@${form.value.domain}`)
  if (!target) return
  callLabel.value = displayNumber.value
  callError.value = ''
  callState.value = 'dialing'
  displayNumber.value = ''
  const inviter = new Inviter(ua.value, target)
  attachSession(inviter, false)
  try {
    await inviter.invite({
      sessionDescriptionHandlerOptions: { constraints: { audio: true, video: false } },
    })
  } catch (error) {
    callError.value = callFailureMessage(error)
    callState.value = 'idle'
    currentSession.value = null
  }
}

// ── Incoming call actions ─────────────────────────────────────────────────────
async function answerCall() {
  if (!currentSession.value) return
  await currentSession.value.accept({
    sessionDescriptionHandlerOptions: { constraints: { audio: true, video: false } },
  })
}

function rejectCall() {
  if (!currentSession.value) return
  currentSession.value.reject({ statusCode: 486 })
}

// ── In-call actions ───────────────────────────────────────────────────────────
function hangup() {
  if (!currentSession.value) return
  const session = currentSession.value
  callState.value = 'idle'
  currentSession.value = null
  callLabel.value = ''
  if (session.state === SessionState.Established) {
    session.bye()
  } else if (session.state === SessionState.Initial || session.state === SessionState.Establishing) {
    session.cancel?.()
  } else {
    session.reject?.({ statusCode: 487 })
  }
}

function toggleMute() {
  if (!currentSession.value) return
  const sdh = currentSession.value.sessionDescriptionHandler
  if (!sdh) return
  muted.value = !muted.value
  sdh.peerConnection?.getSenders().forEach((s) => {
    if (s.track?.kind === 'audio') s.track.enabled = !muted.value
  })
}

onUnmounted(() => disconnect())
</script>

<style scoped>
.phone-page {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
}

.panel {
  background: #fff;
  border-radius: 16px;
  padding: 28px 24px;
  width: 360px;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.4);
}

.phone-logo {
  font-size: 20px;
  font-weight: bold;
  text-align: center;
  margin-bottom: 20px;
  color: #333;
}

.status-msg.error {
  color: #f56c6c;
  font-size: 13px;
  text-align: center;
  margin-top: 8px;
}

/* Softphone header */
.phone-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  flex-wrap: wrap;
}

.phone-header .phone-logo {
  margin-bottom: 0;
  font-size: 16px;
  flex: 1;
}

.reg-status {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  color: #666;
}

.reg-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  display: inline-block;
}

.dot-green { background: #67c23a; }
.dot-yellow { background: #e6a23c; }

.user-label {
  font-size: 11px;
  color: #999;
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Display */
.display-area {
  background: #f5f7fa;
  border-radius: 10px;
  padding: 14px 16px;
  margin-bottom: 12px;
  min-height: 60px;
}

.display-number {
  font-size: 26px;
  font-family: 'Courier New', monospace;
  letter-spacing: 2px;
  color: #222;
  min-height: 32px;
}

.call-state-label {
  font-size: 13px;
  color: #67c23a;
  margin-top: 4px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.timer { font-size: 12px; color: #999; }

.call-error {
  color: #f56c6c;
  font-size: 13px;
  margin-top: 4px;
}

/* Incoming panel */
.incoming-panel {
  background: #f0f9eb;
  border-radius: 10px;
  padding: 14px;
  margin-bottom: 12px;
  text-align: center;
}

.incoming-label {
  font-size: 15px;
  color: #333;
  margin-bottom: 12px;
}

/* Dialpad */
.dialpad { margin-bottom: 4px; }

.key-row {
  display: flex;
  gap: 8px;
  margin-bottom: 8px;
  justify-content: center;
}

.key-btn {
  flex: 1;
  height: 56px;
  border: 1px solid #ddd;
  border-radius: 10px;
  background: #fff;
  cursor: pointer;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  transition: background 0.1s, transform 0.1s;
  font-family: inherit;
  user-select: none;
}

.key-btn:hover { background: #f0f2f5; }
.key-btn:active { background: #e6e8eb; transform: scale(0.96); }

.digit { font-size: 22px; font-weight: 500; color: #333; line-height: 1; }
.sub { font-size: 9px; color: #999; letter-spacing: 1px; margin-top: 2px; }

.key-call {
  background: #67c23a;
  border-color: #67c23a;
  color: #fff;
  font-size: 22px;
}
.key-call:hover { background: #5daf34; }
.key-call:disabled { background: #c0c4cc; border-color: #c0c4cc; cursor: not-allowed; }

.key-clear { background: #fef0f0; border-color: #fbc4c4; color: #f56c6c; }
.key-clear:hover { background: #fde2e2; }

/* In-call controls */
.incall-controls {
  display: flex;
  justify-content: center;
  gap: 20px;
  padding: 8px 0;
}

.btn-row {
  display: flex;
  justify-content: center;
  gap: 24px;
}
</style>
