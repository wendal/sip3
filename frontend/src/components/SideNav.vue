<template>
  <div class="side-nav" :class="{ 'side-nav--collapsed': collapsed }">
    <div class="side-nav__brand">
      <div class="brand-mark">
        <el-icon><PhoneFilled /></el-icon>
      </div>
      <transition name="brand-text">
        <span v-if="!collapsed" class="brand-name">SIP3 Admin</span>
      </transition>
    </div>

    <nav class="side-nav__list">
      <router-link
        v-for="item in items"
        :key="item.path"
        :to="item.path"
        class="nav-item"
        :class="{ 'is-active': route.path.startsWith(item.path) }"
        :title="collapsed ? item.label : ''"
        @click="$emit('navigate')"
      >
        <span class="nav-item__bar" />
        <el-icon class="nav-item__icon"><component :is="item.icon" /></el-icon>
        <span v-if="!collapsed" class="nav-item__label">{{ item.label }}</span>
      </router-link>
    </nav>

    <div v-if="!collapsed" class="side-nav__footer">
      v0.1 · Open Source SIP Server
    </div>
  </div>
</template>

<script setup>
import { useRoute } from 'vue-router'
import {
  DataLine, User, Lock, Monitor, WarningFilled, Setting, PhoneFilled, ChatDotRound,
} from '@element-plus/icons-vue'

defineProps({
  collapsed: { type: Boolean, default: false },
})
defineEmits(['navigate'])

const route = useRoute()

const items = [
  { path: '/dashboard',   label: '控制台',   icon: DataLine },
  { path: '/accounts',    label: 'SIP 账号', icon: User },
  { path: '/conferences', label: '会议室',   icon: ChatDotRound },
  { path: '/acl',         label: 'IP ACL',   icon: Lock },
  { path: '/status',      label: '系统状态', icon: Monitor },
  { path: '/security',    label: '安全监控', icon: WarningFilled },
  { path: '/admin-users', label: '管理员',   icon: Setting },
]
</script>

<style scoped>
.side-nav {
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 14px 12px;
  background: var(--sip-sidebar-bg);
}
.side-nav--collapsed { padding: 14px 8px; }

.side-nav__brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 8px 16px;
  border-bottom: 1px solid var(--sip-divider);
  margin-bottom: 12px;
  min-height: 44px;
}
.brand-mark {
  width: 36px; height: 36px;
  border-radius: 10px;
  background: linear-gradient(135deg, var(--sip-primary) 0%, #5e5ce6 100%);
  color: #fff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 18px;
  flex-shrink: 0;
  box-shadow: 0 4px 12px rgba(10, 132, 255, 0.30);
}
.brand-name {
  font-size: 16px;
  font-weight: 700;
  color: var(--sip-text);
  letter-spacing: -0.3px;
  white-space: nowrap;
}

.side-nav__list {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  overflow-y: auto;
}

.nav-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border-radius: 10px;
  color: var(--sip-text-2);
  font-size: 14px;
  font-weight: 500;
  text-decoration: none;
  transition: background 0.15s, color 0.15s;
  cursor: pointer;
}
.nav-item__bar {
  position: absolute;
  left: -12px;
  top: 8px; bottom: 8px;
  width: 3px;
  border-radius: 0 3px 3px 0;
  background: transparent;
  transition: background 0.15s;
}
.nav-item__icon { font-size: 18px; flex-shrink: 0; }
.nav-item__label { white-space: nowrap; }
.nav-item:hover {
  background: var(--sip-surface-2);
  color: var(--sip-text);
}
.nav-item.is-active {
  background: var(--sip-sidebar-active-bg);
  color: var(--sip-sidebar-active-color);
}
.nav-item.is-active .nav-item__bar { background: var(--sip-primary); }

.side-nav--collapsed .nav-item { justify-content: center; padding: 12px 0; }

.side-nav__footer {
  font-size: 11px;
  color: var(--sip-text-3);
  text-align: center;
  padding: 12px 4px 4px;
  border-top: 1px solid var(--sip-divider);
}

.brand-text-enter-active, .brand-text-leave-active { transition: opacity 0.15s; }
.brand-text-enter-from, .brand-text-leave-to { opacity: 0; }
</style>
