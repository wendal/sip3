<template>
  <div class="login-page">
    <!-- Brand panel -->
    <div class="brand-panel">
      <div class="brand-panel__inner">
        <div class="brand-mark">
          <el-icon><PhoneFilled /></el-icon>
        </div>
        <h1 class="brand-title">SIP3 Server</h1>
        <p class="brand-tag">开源、轻量、现代化的 SIP 服务器</p>
        <ul class="brand-features">
          <li><el-icon><Check /></el-icon> SIP UDP / TLS / WS / WSS 多协议接入</li>
          <li><el-icon><Check /></el-icon> 注册、代理、媒体转发一体化</li>
          <li><el-icon><Check /></el-icon> 内置安全监控与 IP ACL</li>
          <li><el-icon><Check /></el-icon> 浏览器软电话、即时消息</li>
        </ul>
      </div>
      <div class="brand-panel__bg" />
    </div>

    <!-- Login form panel -->
    <div class="form-panel">
      <div class="form-card">
        <div class="form-card__header">
          <h2>欢迎回来</h2>
          <p>请使用管理员账号登录</p>
        </div>

        <el-form :model="form" @submit.prevent="handleLogin" label-position="top" size="large">
          <el-form-item label="用户名">
            <el-input
              v-model="form.username"
              placeholder="admin"
              :prefix-icon="User"
              autocomplete="username"
              @keyup.enter="handleLogin"
            />
          </el-form-item>
          <el-form-item label="密码">
            <el-input
              v-model="form.password"
              type="password"
              placeholder="请输入密码"
              :prefix-icon="Lock"
              show-password
              autocomplete="current-password"
              @keyup.enter="handleLogin"
            />
          </el-form-item>
          <el-form-item>
            <div class="form-row">
              <el-checkbox v-model="remember">记住用户名</el-checkbox>
            </div>
          </el-form-item>
          <el-form-item>
            <el-button
              type="primary"
              size="large"
              style="width: 100%;"
              :loading="loading"
              @click="handleLogin"
            >
              登 录
            </el-button>
          </el-form-item>
        </el-form>

        <div class="form-card__footer">
          <span>需要软电话？</span>
          <a href="/phone" target="_blank">打开 SIP3 软电话 →</a>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage } from 'element-plus'
import { User, Lock, PhoneFilled, Check } from '@element-plus/icons-vue'
import { useAuthStore } from '../store/auth'

const router = useRouter()
const authStore = useAuthStore()

const REMEMBER_KEY = 'sip3-remember-username'
const form = ref({ username: '', password: '' })
const remember = ref(true)
const loading = ref(false)

onMounted(() => {
  try {
    const saved = localStorage.getItem(REMEMBER_KEY)
    if (saved) form.value.username = saved
  } catch { /* ignore */ }
})

const handleLogin = async () => {
  if (!form.value.username || !form.value.password) {
    ElMessage.warning('请输入用户名和密码')
    return
  }
  try {
    loading.value = true
    await authStore.login(form.value.username, form.value.password)
    try {
      if (remember.value) localStorage.setItem(REMEMBER_KEY, form.value.username)
      else localStorage.removeItem(REMEMBER_KEY)
    } catch { /* ignore */ }
    ElMessage.success('登录成功')
    router.push('/dashboard')
  } catch (e) {
    const msg = e.response?.data || e.message || '登录失败'
    ElMessage.error(typeof msg === 'string' ? msg : '用户名或密码错误')
  } finally {
    loading.value = false
  }
}
</script>

<style scoped>
.login-page {
  min-height: 100vh;
  display: flex;
  background: var(--sip-bg);
}

/* Brand side */
.brand-panel {
  position: relative;
  flex: 1.2;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 48px;
  color: #fff;
  overflow: hidden;
  background: linear-gradient(135deg, #0a84ff 0%, #5e5ce6 50%, #af52de 100%);
}
.brand-panel__bg {
  position: absolute;
  inset: 0;
  background:
    radial-gradient(circle at 20% 20%, rgba(255,255,255,0.18) 0, transparent 35%),
    radial-gradient(circle at 80% 80%, rgba(255,255,255,0.10) 0, transparent 40%);
  pointer-events: none;
}
.brand-panel__inner {
  position: relative;
  z-index: 1;
  max-width: 460px;
}
.brand-mark {
  width: 64px; height: 64px;
  border-radius: 18px;
  background: rgba(255,255,255,0.18);
  backdrop-filter: blur(10px);
  display: flex; align-items: center; justify-content: center;
  font-size: 32px;
  margin-bottom: 28px;
}
.brand-title {
  font-size: 38px;
  font-weight: 700;
  letter-spacing: -1px;
  margin: 0 0 12px;
}
.brand-tag {
  font-size: 16px;
  opacity: 0.9;
  margin: 0 0 32px;
}
.brand-features {
  list-style: none;
  padding: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
  font-size: 14px;
  opacity: 0.95;
}
.brand-features li {
  display: flex;
  align-items: center;
  gap: 10px;
}
.brand-features .el-icon {
  background: rgba(255,255,255,0.18);
  padding: 4px;
  border-radius: 6px;
  font-size: 14px;
}

/* Form side */
.form-panel {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 32px;
  min-width: 360px;
}
.form-card {
  width: 100%;
  max-width: 380px;
}
.form-card__header { text-align: left; margin-bottom: 28px; }
.form-card__header h2 {
  margin: 0 0 6px;
  font-size: 26px;
  font-weight: 700;
  color: var(--sip-text);
  letter-spacing: -0.5px;
}
.form-card__header p {
  margin: 0;
  color: var(--sip-text-2);
  font-size: 14px;
}
.form-row { width: 100%; display: flex; justify-content: flex-start; }
.form-card__footer {
  margin-top: 16px;
  font-size: 13px;
  color: var(--sip-text-2);
  text-align: center;
}
.form-card__footer a {
  color: var(--sip-primary);
  margin-left: 4px;
  font-weight: 500;
}

@media (max-width: 900px) {
  .brand-panel { display: none; }
  .form-panel { padding: 24px; }
}
</style>
