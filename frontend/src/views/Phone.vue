<template>
  <div class="phone-page" :class="{ 'phone-page--mobile': isMobile }">
    <!-- iPhone-style frame on desktop, full screen on mobile -->
    <div class="phone-frame" :class="{ 'phone-frame--bare': isMobile }">
      <div class="phone-notch" v-if="!isMobile" />

      <!-- Status bar -->
      <div class="status-bar" v-if="ua">
        <span class="status-bar__time num">{{ clockTime }}</span>
        <span class="status-bar__center">
          <span :class="['signal-dot', registered ? 'signal-dot--ok' : 'signal-dot--pending']" />
          {{ registered ? `SIP3 · ${form.username}` : '注册中…' }}
        </span>
        <span class="status-bar__right">
          <el-icon><Connection /></el-icon>
        </span>
      </div>

      <div class="phone-content">
        <!-- ─── Login panel ───────────────────────────────────────────── -->
        <transition name="slide-fade">
          <section v-if="!ua" key="login" class="screen screen--login">
            <div class="login-hero">
              <div class="login-hero__mark"><el-icon><PhoneFilled /></el-icon></div>
              <h1 class="login-hero__title">SIP3 软电话</h1>
              <p class="login-hero__sub">使用 SIP 账号登录开始呼叫</p>
            </div>

            <el-form :model="form" @submit.prevent="connect" class="login-form" label-position="top">
              <el-form-item label="用户名">
                <el-input v-model="form.username" placeholder="如 1001" autocomplete="username" />
              </el-form-item>
              <el-form-item label="密码">
                <el-input v-model="form.password" type="password" show-password
                  placeholder="SIP 账号密码" autocomplete="current-password" />
              </el-form-item>

              <el-collapse v-model="advancedOpen" class="login-advanced">
                <el-collapse-item title="高级设置" name="adv">
                  <el-form-item label="域名">
                    <el-input v-model="form.domain" placeholder="sip.example.com" />
                  </el-form-item>
                  <el-form-item label="WSS 服务器">
                    <el-input v-model="form.wssServer" placeholder="wss://sip.example.com:5443" />
                  </el-form-item>
                </el-collapse-item>
              </el-collapse>

              <el-button
                type="primary" size="large" native-type="submit"
                :loading="connecting" class="login-submit"
              >
                连接并注册
              </el-button>
              <div v-if="statusMsg" class="status-msg-error">{{ statusMsg }}</div>
            </el-form>
          </section>

          <!-- ─── Incoming call (full-screen) ─────────────────────────── -->
          <section v-else-if="callState === 'incoming'" key="incoming" class="screen screen--incoming">
            <div class="incoming-meta">
              <div class="incoming-label">来电</div>
              <div class="incoming-avatar">{{ avatarChar(callLabel) }}</div>
              <div class="incoming-name">{{ callLabel || '未知' }}</div>
              <div class="incoming-sub">SIP3 · {{ form.domain }}</div>
            </div>
            <div class="incoming-actions">
              <button class="round-btn round-btn--danger pulse" @click="rejectCall">
                <el-icon><CloseBold /></el-icon>
              </button>
              <button class="round-btn round-btn--success pulse" @click="answerCall">
                <el-icon><PhoneFilled /></el-icon>
              </button>
            </div>
          </section>

          <!-- ─── Active call ────────────────────────────────────────── -->
          <section v-else-if="activeCallStates.includes(callState)" key="active" class="screen screen--active">
            <div class="active-meta">
              <div class="active-avatar">{{ avatarChar(callLabel) }}</div>
              <div class="active-name">{{ callLabel || '通话中' }}</div>
              <div class="active-sub">{{ callStateLabel }} <span v-if="callState === 'active'" class="num">· {{ formatDuration(callDuration) }}</span></div>
            </div>

            <div class="active-actions">
              <button class="round-btn round-btn--ghost" :class="{ 'is-on': muted }" @click="toggleMute" :title="muted ? '取消静音' : '静音'">
                <el-icon><Microphone /></el-icon>
              </button>
              <div class="round-btn-spacer" />
              <button class="round-btn round-btn--danger" @click="hangup" title="挂断">
                <el-icon><CloseBold /></el-icon>
              </button>
            </div>
          </section>

          <!-- ─── Idle / Dialing — keypad / messages ─────────────────── -->
          <section v-else key="idle" class="screen screen--idle">
            <div v-if="activeTab === 'keypad'" class="tab-keypad">
              <div class="display-area">
                <div class="display-number num">{{ displayNumber || '\u00a0' }}</div>
                <div v-if="callError" class="display-error">{{ callError }}</div>
              </div>

              <div class="dialpad">
                <button
                  v-for="key in DIALPAD_KEYS_FLAT"
                  :key="key.digit"
                  class="dial-key"
                  @click="pressKey(key.digit)"
                >
                  <span class="dial-key__digit">{{ key.digit }}</span>
                  <span class="dial-key__sub">{{ key.sub }}</span>
                </button>
              </div>

              <div class="action-row">
                <div class="action-row__spacer" />
                <button
                  class="round-btn round-btn--success round-btn--large"
                  :disabled="!displayNumber || callState === 'dialing'"
                  @click="makeCall"
                  title="呼叫"
                >
                  <el-icon><PhoneFilled /></el-icon>
                </button>
                <button
                  class="action-row__back"
                  v-if="displayNumber"
                  @click="displayNumber = displayNumber.slice(0, -1)"
                  title="退格"
                >
                  <el-icon><Back /></el-icon>
                </button>
                <div class="action-row__spacer" v-if="!displayNumber" />
              </div>
            </div>

            <div v-else-if="activeTab === 'messages'" class="tab-messages">
              <div class="chat-target">
                <el-input
                  v-model="chatTarget"
                  placeholder="对方分机号 (如 1002)"
                  clearable
                  size="default"
                />
                <el-button :loading="chatLoading" @click="loadChatHistory">历史</el-button>
              </div>

              <div class="chat-list" ref="chatListEl">
                <EmptyState v-if="!chatMessages.length" title="暂无消息" subtitle="从这里开始对话" :icon="ChatDotRound" />
                <div
                  v-for="msg in chatMessages"
                  :key="msg.id"
                  :class="['chat-row', msg.direction === 'out' ? 'chat-row--out' : 'chat-row--in']"
                >
                  <div class="chat-bubble">{{ msg.content }}</div>
                  <div class="chat-meta">
                    <span class="num">{{ formatChatTime(msg.at) }}</span>
                    <span v-if="msg.direction === 'out'" class="chat-status">
                      {{ msg.status === 'failed' ? '✗ 失败' : (msg.status === 'sending' ? '…发送中' : '✓ 已送达') }}
                    </span>
                  </div>
                </div>
              </div>

              <div class="chat-send">
                <el-input
                  v-model="chatInput"
                  placeholder="输入消息，回车发送"
                  @keyup.enter="sendMessage"
                  :disabled="!chatTarget.trim()"
                />
                <button class="send-btn" :disabled="!chatTarget.trim() || !chatInput.trim()" @click="sendMessage">
                  <el-icon><Promotion /></el-icon>
                </button>
              </div>
              <div v-if="chatError" class="chat-error">{{ chatError }}</div>
            </div>
          </section>
        </transition>
      </div>

      <!-- Tab Bar (only when registered & not in active/incoming call) -->
      <nav v-if="ua && !activeCallStates.includes(callState) && callState !== 'incoming'" class="tab-bar">
        <button :class="['tab-bar__item', activeTab === 'keypad' ? 'is-active' : '']" @click="activeTab = 'keypad'">
          <el-icon><PhoneFilled /></el-icon>
          <span>键盘</span>
        </button>
        <button :class="['tab-bar__item', activeTab === 'messages' ? 'is-active' : '']" @click="activeTab = 'messages'">
          <el-icon><ChatDotRound /></el-icon>
          <span>消息</span>
        </button>
        <button class="tab-bar__item" @click="disconnect" title="断开并退出">
          <el-icon><SwitchButton /></el-icon>
          <span>断开</span>
        </button>
      </nav>

      <audio ref="remoteAudio" autoplay />
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, nextTick, watch } from 'vue'
import {
  PhoneFilled, CloseBold, Microphone, ChatDotRound, Back, Promotion,
  Connection, SwitchButton,
} from '@element-plus/icons-vue'
import { activeCallStates, callFailureMessage } from '../utils/callUiState.mjs'
import { DIALPAD_KEYS } from '../utils/dialpad.mjs'
import {
  UserAgent, Registerer, RegistererState, Inviter, Messager, SessionState,
} from 'sip.js'
import { useBreakpoint } from '../composables/useBreakpoint'
import EmptyState from '../components/EmptyState.vue'

const { isMobile } = useBreakpoint()

// Flatten the 3-row dialpad to a single grid for easier CSS layout.
const DIALPAD_KEYS_FLAT = DIALPAD_KEYS.flat()

// ── Form state ────────────────────────────────────────────────────────────────
const form = ref({
  username: '',
  password: '',
  domain: window.location.hostname,
  wssServer: `wss://${window.location.hostname}:5443`,
})
const advancedOpen = ref([])

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
const chatTarget = ref('')
const chatInput = ref('')
const chatMessages = ref([])
const chatLoading = ref(false)
const chatError = ref('')
const chatListEl = ref(null)
const activeTab = ref('keypad') // keypad | messages
const clockTime = ref('')

let callTimer = null
let clockTimer = null

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

function formatChatTime(value) {
  if (!value) return '-'
  const d = new Date(value)
  if (Number.isNaN(d.getTime())) return value
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

function avatarChar(s) {
  return (s || '?').slice(0, 1).toUpperCase()
}

function pressKey(digit) {
  displayNumber.value += digit
  // Subtle haptic on supported devices.
  try { navigator.vibrate?.(12) } catch { /* ignore */ }
}

function appendChatMessage({ direction, peer, content, at, status }) {
  chatMessages.value.push({
    id: `${Date.now()}-${Math.random()}`,
    direction,
    peer: peer || '未知',
    content: content || '',
    at: at || new Date().toISOString(),
    status: status || 'delivered',
  })
}

// Auto-scroll chat to the bottom whenever a new message arrives.
watch(() => chatMessages.value.length, async () => {
  await nextTick()
  if (chatListEl.value) chatListEl.value.scrollTop = chatListEl.value.scrollHeight
})

async function loadChatHistory() {
  const peer = chatTarget.value.trim()
  if (!peer) {
    chatError.value = '请先输入聊天分机号'
    return
  }
  chatLoading.value = true
  chatError.value = ''
  try {
    const resp = await fetch('/api/messages/history', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        username: form.value.username,
        password: form.value.password,
        domain: form.value.domain,
        peer,
        limit: 100,
      }),
    })
    const payload = await resp.json().catch(() => ({}))
    if (!resp.ok) {
      throw new Error(payload?.message || payload?.error || '加载消息历史失败')
    }
    const selfAor = `${form.value.username}@${form.value.domain}`.toLowerCase()
    const rows = (payload.data || []).slice().reverse()
    chatMessages.value = rows.map((row, idx) => ({
      id: `${row.id || 'history'}-${idx}`,
      direction: (row.sender || '').toLowerCase() === selfAor ? 'out' : 'in',
      peer,
      content: row.body || '',
      at: row.created_at,
      status: row.status || 'delivered',
    }))
  } catch (e) {
    chatError.value = e?.message || '加载消息历史失败'
  } finally {
    chatLoading.value = false
  }
}

async function sendMessage() {
  if (!ua.value) return
  const peer = chatTarget.value.trim()
  const content = chatInput.value.trim()
  if (!peer || !content) {
    chatError.value = '请输入聊天分机号和消息内容'
    return
  }
  const target = UserAgent.makeURI(`sip:${peer}@${form.value.domain}`)
  if (!target) {
    chatError.value = '聊天分机号格式无效'
    return
  }

  chatError.value = ''
  const optimistic = {
    id: `${Date.now()}-${Math.random()}`,
    direction: 'out',
    peer,
    content,
    at: new Date().toISOString(),
    status: 'sending',
  }
  chatMessages.value.push(optimistic)
  chatInput.value = ''

  try {
    const messager = new Messager(ua.value, target, content, 'text/plain')
    await messager.message()
    optimistic.status = 'delivered'
  } catch (e) {
    optimistic.status = 'failed'
    chatError.value = `发送失败：${e?.message || '未知错误'}`
  }
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
      { urls: data.uris, username: data.username, credential: data.password },
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
        onMessage(message) {
          const peer = message.request.from?.uri?.user || '未知'
          if (!chatTarget.value) {
            chatTarget.value = peer
          }
          appendChatMessage({
            direction: 'in',
            peer,
            content: message.request.body,
            status: 'delivered',
          })
          message.accept().catch(() => {})
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
    try { currentSession.value.bye() } catch { /* ignore */ }
  }
  if (registerer.value) {
    try { await registerer.value.unregister() } catch { /* ignore */ }
  }
  if (ua.value) {
    try { await ua.value.stop() } catch { /* ignore */ }
  }
  ua.value = null
  registerer.value = null
  registered.value = false
  callState.value = 'idle'
  callLabel.value = ''
  stopCallTimer()
  chatTarget.value = ''
  chatInput.value = ''
  chatMessages.value = []
  chatError.value = ''
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

function tickClock() {
  clockTime.value = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

onMounted(() => {
  tickClock()
  clockTimer = setInterval(tickClock, 30_000)
})

onUnmounted(() => {
  if (clockTimer) clearInterval(clockTimer)
  disconnect()
})
</script>

<style scoped>
/* ─── Page background (visible only on desktop around the iPhone frame) ─── */
.phone-page {
  min-height: 100vh;
  min-height: 100dvh;
  display: flex;
  align-items: center;
  justify-content: center;
  background:
    radial-gradient(circle at 20% 20%, rgba(94, 92, 230, 0.35) 0%, transparent 50%),
    radial-gradient(circle at 80% 80%, rgba(10, 132, 255, 0.30) 0%, transparent 50%),
    linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
  padding: 20px;
}
.phone-page--mobile { padding: 0; background: var(--sip-bg); }

/* ─── iPhone frame ─── */
.phone-frame {
  width: 390px;
  height: 760px;
  border-radius: 48px;
  background: #000;
  padding: 14px;
  box-shadow:
    0 0 0 2px #1a1a1a,
    0 0 0 12px #2a2a2a,
    0 30px 80px rgba(0, 0, 0, 0.6);
  position: relative;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.phone-frame--bare {
  width: 100vw;
  height: 100vh;
  height: 100dvh;
  border-radius: 0;
  padding: 0;
  box-shadow: none;
  background: #000;
}

/* Inner screen surface */
.phone-frame::after {
  content: '';
  position: absolute;
  inset: 14px;
  border-radius: 36px;
  background: #f2f2f7;
  z-index: 0;
  pointer-events: none;
}
.phone-frame--bare::after { inset: 0; border-radius: 0; }

.phone-notch {
  position: absolute;
  top: 14px;
  left: 50%;
  transform: translateX(-50%);
  width: 130px; height: 28px;
  background: #000;
  border-radius: 0 0 18px 18px;
  z-index: 3;
}

.phone-content {
  position: relative;
  z-index: 2;
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding-top: 44px; /* leave room for status bar */
}
.phone-frame--bare .phone-content { padding-top: env(safe-area-inset-top, 24px); }

/* ─── Status bar ─── */
.status-bar {
  position: absolute;
  top: 14px;
  left: 14px;
  right: 14px;
  height: 44px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 24px 0 28px;
  font-size: 13px;
  font-weight: 600;
  color: var(--sip-text);
  z-index: 3;
}
.phone-frame--bare .status-bar {
  top: env(safe-area-inset-top, 0);
  left: 0; right: 0;
  padding: 0 16px;
}
.status-bar__center {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--sip-text-2);
  font-weight: 500;
}
.signal-dot {
  width: 8px; height: 8px;
  border-radius: 50%;
  display: inline-block;
}
.signal-dot--ok { background: var(--sip-success); }
.signal-dot--pending { background: var(--sip-warning); animation: blink 1.4s infinite; }
@keyframes blink { 50% { opacity: 0.35; } }
.status-bar__right { color: var(--sip-text-2); display: inline-flex; }

/* ─── Screen container ─── */
.screen {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 12px 18px 0;
  overflow: hidden;
}

/* ─── Login screen ─── */
.screen--login { padding: 24px 24px 0; overflow-y: auto; }
.login-hero { text-align: center; margin: 12px 0 18px; }
.login-hero__mark {
  width: 64px; height: 64px;
  margin: 0 auto 12px;
  border-radius: 18px;
  background: linear-gradient(135deg, var(--sip-primary), #5e5ce6);
  color: #fff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 30px;
  box-shadow: 0 8px 24px rgba(10, 132, 255, 0.35);
}
.login-hero__title { margin: 0; font-size: 22px; font-weight: 700; color: var(--sip-text); }
.login-hero__sub { margin: 4px 0 0; color: var(--sip-text-2); font-size: 13px; }
.login-form { padding-bottom: 24px; }
.login-advanced :deep(.el-collapse) { border: none; background: transparent; }
.login-advanced :deep(.el-collapse-item__header),
.login-advanced :deep(.el-collapse-item__wrap) {
  background: transparent !important;
  border: none !important;
}
.login-submit { width: 100%; margin-top: 8px; height: 46px; font-size: 15px; }
.status-msg-error { color: var(--sip-danger); font-size: 13px; text-align: center; margin-top: 12px; }

/* ─── Incoming screen ─── */
.screen--incoming {
  background: linear-gradient(180deg, #1c1c1e 0%, #000 100%);
  margin: -12px -18px 0;
  padding: 60px 24px 36px;
  border-radius: 24px 24px 0 0;
  color: #fff;
  align-items: center;
}
.incoming-meta { text-align: center; margin-top: auto; }
.incoming-label { font-size: 13px; color: rgba(255,255,255,0.6); margin-bottom: 24px; }
.incoming-avatar {
  width: 110px; height: 110px;
  border-radius: 50%;
  background: linear-gradient(135deg, var(--sip-primary), #5e5ce6);
  margin: 0 auto 18px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 44px;
  font-weight: 600;
  color: #fff;
  box-shadow: 0 0 0 8px rgba(255,255,255,0.05);
}
.incoming-name { font-size: 28px; font-weight: 600; }
.incoming-sub { font-size: 13px; color: rgba(255,255,255,0.5); margin-top: 4px; }

.incoming-actions {
  margin-top: auto;
  display: flex;
  align-items: center;
  justify-content: space-around;
  width: 100%;
  padding: 36px 24px 12px;
}

/* ─── Active call screen ─── */
.screen--active {
  background: linear-gradient(180deg, #2c2c2e 0%, #000 100%);
  margin: -12px -18px 0;
  padding: 48px 24px 36px;
  border-radius: 24px 24px 0 0;
  color: #fff;
  align-items: center;
}
.active-meta { text-align: center; margin-top: 24px; }
.active-avatar {
  width: 96px; height: 96px;
  border-radius: 50%;
  background: linear-gradient(135deg, var(--sip-primary), #5e5ce6);
  margin: 0 auto 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 38px;
  font-weight: 600;
  color: #fff;
}
.active-name { font-size: 24px; font-weight: 600; }
.active-sub { font-size: 14px; color: rgba(255,255,255,0.6); margin-top: 6px; }

.active-actions {
  margin-top: auto;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 20px;
  padding: 0 24px 12px;
  width: 100%;
}
.round-btn-spacer { flex: 0 0 12px; }

/* ─── Idle / keypad screen ─── */
.screen--idle { padding: 8px 18px 0; }
.tab-keypad, .tab-messages {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.display-area {
  min-height: 70px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 8px 0 12px;
}
.display-number {
  font-size: 38px;
  font-weight: 300;
  letter-spacing: 1px;
  color: var(--sip-text);
  line-height: 1;
}
.display-error {
  color: var(--sip-danger);
  font-size: 13px;
  margin-top: 6px;
}

/* Dialpad — 3 column grid */
.dialpad {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
  padding: 8px 12px;
  margin-top: 4px;
}
.dial-key {
  aspect-ratio: 1 / 1;
  border-radius: 50%;
  border: none;
  background: rgba(118, 118, 128, 0.20);
  color: var(--sip-text);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: background 0.12s, transform 0.08s;
  font-family: inherit;
  user-select: none;
  -webkit-tap-highlight-color: transparent;
}
.dial-key:hover  { background: rgba(118, 118, 128, 0.32); }
.dial-key:active { background: rgba(118, 118, 128, 0.40); transform: scale(0.94); }
.dial-key__digit {
  font-size: 30px;
  font-weight: 400;
  line-height: 1;
  letter-spacing: 1px;
}
.dial-key__sub {
  font-size: 9px;
  font-weight: 600;
  color: var(--sip-text-2);
  letter-spacing: 2px;
  margin-top: 2px;
  height: 10px;
}

.action-row {
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  padding: 10px 12px 14px;
  gap: 12px;
}
.action-row__spacer { width: 100%; }
.action-row__back {
  border: none;
  background: transparent;
  color: var(--sip-text-2);
  font-size: 22px;
  cursor: pointer;
  width: 56px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.action-row__back:hover { color: var(--sip-text); }

/* ─── Round buttons (call / hangup) ─── */
.round-btn {
  width: 64px; height: 64px;
  border-radius: 50%;
  border: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 26px;
  cursor: pointer;
  transition: transform 0.1s, box-shadow 0.15s, background 0.15s;
  -webkit-tap-highlight-color: transparent;
}
.round-btn:active { transform: scale(0.93); }
.round-btn--large { width: 72px; height: 72px; font-size: 30px; }
.round-btn--success {
  background: var(--sip-success); color: #fff;
  box-shadow: 0 6px 18px rgba(52, 199, 89, 0.40);
}
.round-btn--success:disabled {
  background: rgba(118,118,128,0.30); color: rgba(255,255,255,0.6);
  box-shadow: none; cursor: not-allowed;
}
.round-btn--danger {
  background: var(--sip-danger); color: #fff;
  box-shadow: 0 6px 18px rgba(255, 59, 48, 0.40);
}
.round-btn--ghost {
  background: rgba(255,255,255,0.12); color: #fff;
}
.round-btn--ghost.is-on { background: var(--sip-warning); color: #fff; }

.pulse {
  animation: pulse 1.6s infinite;
}
@keyframes pulse {
  0%, 100% { box-shadow: 0 6px 18px rgba(255, 59, 48, 0.40); }
  50%      { box-shadow: 0 6px 28px rgba(255, 59, 48, 0.70); }
}
.round-btn--success.pulse {
  animation: pulseGreen 1.6s infinite;
}
@keyframes pulseGreen {
  0%, 100% { box-shadow: 0 6px 18px rgba(52, 199, 89, 0.40); }
  50%      { box-shadow: 0 6px 28px rgba(52, 199, 89, 0.70); }
}

/* ─── Messages tab ─── */
.tab-messages {
  padding-top: 4px;
  padding-bottom: 8px;
  gap: 8px;
}
.chat-target {
  display: flex;
  gap: 8px;
  padding: 0 2px;
}
.chat-target :deep(.el-input) { flex: 1; }
.chat-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px 4px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.chat-row { display: flex; flex-direction: column; max-width: 78%; }
.chat-row--out { align-self: flex-end; align-items: flex-end; }
.chat-row--in  { align-self: flex-start; align-items: flex-start; }
.chat-bubble {
  padding: 8px 12px;
  border-radius: 18px;
  font-size: 14px;
  word-break: break-word;
  line-height: 1.4;
}
.chat-row--out .chat-bubble {
  background: var(--sip-primary);
  color: #fff;
  border-bottom-right-radius: 6px;
}
.chat-row--in .chat-bubble {
  background: rgba(118, 118, 128, 0.18);
  color: var(--sip-text);
  border-bottom-left-radius: 6px;
}
.chat-meta {
  font-size: 10px;
  color: var(--sip-text-3);
  margin-top: 2px;
  display: flex;
  gap: 8px;
}
.chat-status { font-weight: 500; }
.chat-send {
  display: flex;
  gap: 8px;
  padding: 4px 2px 0;
  align-items: center;
}
.chat-send :deep(.el-input) { flex: 1; }
.send-btn {
  width: 36px; height: 36px;
  border-radius: 50%;
  border: none;
  background: var(--sip-primary);
  color: #fff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: 16px;
  transition: background 0.15s;
}
.send-btn:hover:not(:disabled) { background: var(--sip-primary-hover); }
.send-btn:disabled {
  background: rgba(118,118,128,0.30);
  cursor: not-allowed;
}
.chat-error { color: var(--sip-danger); font-size: 12px; padding: 0 4px; }

/* ─── Tab bar ─── */
.tab-bar {
  position: relative;
  z-index: 2;
  display: flex;
  justify-content: space-around;
  align-items: stretch;
  background: rgba(255,255,255,0.92);
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border-top: 1px solid rgba(60,60,67,0.18);
  padding: 6px 0 calc(8px + env(safe-area-inset-bottom, 0));
}
.tab-bar__item {
  flex: 1;
  border: none;
  background: transparent;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 6px 0;
  font-size: 10px;
  color: var(--sip-text-2);
  cursor: pointer;
  font-family: inherit;
  -webkit-tap-highlight-color: transparent;
}
.tab-bar__item .el-icon { font-size: 20px; }
.tab-bar__item.is-active { color: var(--sip-primary); }
.tab-bar__item:active { transform: scale(0.94); }

/* Transitions */
.slide-fade-enter-active, .slide-fade-leave-active {
  transition: opacity 0.18s ease, transform 0.18s ease;
}
.slide-fade-enter-from, .slide-fade-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
.slide-fade-leave-active { position: absolute; inset: 0; }
</style>
