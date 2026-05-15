<template>
  <!-- Authenticated layout -->
  <template v-if="authStore.isAuthenticated">
    <el-container class="app-shell">
      <!-- Mobile drawer sidebar -->
      <el-drawer
        v-if="isMobile"
        v-model="drawerOpen"
        direction="ltr"
        :with-header="false"
        size="260px"
        class="app-sidebar-drawer"
      >
        <SideNav :collapsed="false" @navigate="drawerOpen = false" />
      </el-drawer>

      <!-- Desktop sidebar -->
      <el-aside v-if="!isMobile" :width="collapsed ? '72px' : '240px'" class="app-sidebar">
        <SideNav :collapsed="collapsed" />
      </el-aside>

      <el-container>
        <el-header class="app-header">
          <div class="app-header__left">
            <button class="hamburger" @click="toggleSidebar" :title="collapsed || isMobile ? '展开' : '折叠'">
              <el-icon><Fold v-if="!collapsed && !isMobile" /><Expand v-else /></el-icon>
            </button>
            <h1 class="app-header__title">SIP3 Server Management</h1>
          </div>

          <div class="app-header__right">
            <a class="header-link" href="/phone" target="_blank" title="打开软电话">
              <el-icon><PhoneFilled /></el-icon>
              <span class="hide-sm">软电话</span>
            </a>
            <ThemeToggle />
            <el-dropdown trigger="click">
              <div class="user-chip">
                <div class="user-chip__avatar">{{ avatarChar }}</div>
                <span class="user-chip__name hide-sm">{{ authStore.username }}</span>
                <el-icon class="hide-sm"><ArrowDown /></el-icon>
              </div>
              <template #dropdown>
                <el-dropdown-menu>
                  <el-dropdown-item disabled>
                    <el-icon><User /></el-icon>{{ authStore.username }}
                  </el-dropdown-item>
                  <el-dropdown-item divided @click="handleLogout">
                    <el-icon><SwitchButton /></el-icon>退出登录
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
          </div>
        </el-header>

        <el-main class="app-main">
          <div class="app-main__inner">
            <router-view v-slot="{ Component }">
              <transition name="fade" mode="out-in">
                <component :is="Component" />
              </transition>
            </router-view>
          </div>
        </el-main>
      </el-container>
    </el-container>
  </template>

  <!-- Unauthenticated -->
  <template v-else>
    <router-view />
  </template>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage } from 'element-plus'
import {
  Fold, Expand, ArrowDown, User, SwitchButton, PhoneFilled,
} from '@element-plus/icons-vue'
import { useAuthStore } from './store/auth'
import { useBreakpoint } from './composables/useBreakpoint'
import ThemeToggle from './components/ThemeToggle.vue'
import SideNav from './components/SideNav.vue'

const router = useRouter()
const authStore = useAuthStore()
const { isMobile, isTablet } = useBreakpoint()

const collapsed = ref(false)
const drawerOpen = ref(false)

// Auto-collapse on tablet, expand on desktop.
watch(isTablet, (v) => { collapsed.value = v }, { immediate: true })

function toggleSidebar() {
  if (isMobile.value) {
    drawerOpen.value = !drawerOpen.value
  } else {
    collapsed.value = !collapsed.value
  }
}

const avatarChar = computed(() => (authStore.username || '?').slice(0, 1).toUpperCase())

function handleLogout() {
  authStore.logout()
  ElMessage.success('已退出登录')
  router.push('/login')
}
</script>

<style>
.app-shell { height: 100vh; background: var(--sip-bg); }

.app-sidebar {
  background: var(--sip-sidebar-bg) !important;
  border-right: 1px solid var(--sip-divider);
  transition: width 0.2s ease;
  overflow: hidden;
}

.app-sidebar-drawer .el-drawer__body {
  padding: 0;
  background: var(--sip-sidebar-bg);
}

.app-header {
  background: var(--sip-header-bg) !important;
  backdrop-filter: saturate(180%) blur(14px);
  -webkit-backdrop-filter: saturate(180%) blur(14px);
  border-bottom: 1px solid var(--sip-divider);
  display: flex !important;
  align-items: center;
  justify-content: space-between;
  padding: 0 20px !important;
  height: 60px !important;
  position: sticky;
  top: 0;
  z-index: 10;
}

.app-header__left {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
}

.app-header__title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: var(--sip-text);
  letter-spacing: -0.2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.app-header__right {
  display: flex;
  align-items: center;
  gap: 10px;
}

.hamburger {
  width: 36px;
  height: 36px;
  border-radius: 10px;
  border: none;
  background: transparent;
  color: var(--sip-text-2);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: 18px;
  transition: background 0.15s;
}
.hamburger:hover { background: var(--sip-surface-2); color: var(--sip-text); }

.header-link {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border-radius: 999px;
  font-size: 13px;
  color: var(--sip-text-2);
  background: var(--sip-surface-2);
  transition: background 0.15s, color 0.15s;
}
.header-link:hover { background: var(--sip-primary-soft); color: var(--sip-primary); }

.user-chip {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 4px 10px 4px 4px;
  border-radius: 999px;
  background: var(--sip-surface-2);
  cursor: pointer;
  font-size: 13px;
  color: var(--sip-text);
  transition: background 0.15s;
}
.user-chip:hover { background: var(--sip-border); }
.user-chip__avatar {
  width: 28px; height: 28px;
  border-radius: 50%;
  background: linear-gradient(135deg, var(--sip-primary) 0%, #5e5ce6 100%);
  color: #fff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 600;
}
.user-chip__name { font-weight: 500; }

.app-main {
  background: var(--sip-bg) !important;
  padding: 20px !important;
}
.app-main__inner {
  max-width: 1440px;
  margin: 0 auto;
}

.fade-enter-active, .fade-leave-active { transition: opacity 0.18s ease; }
.fade-enter-from, .fade-leave-to { opacity: 0; }

@media (max-width: 768px) {
  .app-main { padding: 14px !important; }
  .app-header { padding: 0 12px !important; }
  .app-header__title { font-size: 14px; }
  .hide-sm { display: none !important; }
}
</style>
