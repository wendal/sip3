<template>
  <!-- Show full layout only when authenticated; Login page renders standalone -->
  <template v-if="authStore.isAuthenticated">
    <el-container style="height: 100vh;">
      <el-aside width="220px" style="background: #001529;">
        <div class="logo">
          <span>SIP3 Admin</span>
        </div>
        <el-menu
          :default-active="$route.path"
          router
          background-color="#001529"
          text-color="#fff"
          active-text-color="#409eff"
        >
          <el-menu-item index="/dashboard">
            <el-icon><DataLine /></el-icon>
            <span>Dashboard</span>
          </el-menu-item>
          <el-menu-item index="/accounts">
            <el-icon><User /></el-icon>
            <span>SIP 账号</span>
          </el-menu-item>
          <el-menu-item index="/acl">
            <el-icon><Lock /></el-icon>
            <span>IP ACL</span>
          </el-menu-item>
          <el-menu-item index="/status">
            <el-icon><Monitor /></el-icon>
            <span>系统状态</span>
          </el-menu-item>
          <el-menu-item index="/security">
            <el-icon><WarningFilled /></el-icon>
            <span>安全监控</span>
          </el-menu-item>
          <el-menu-item index="/admin-users">
            <el-icon><Setting /></el-icon>
            <span>管理员账号</span>
          </el-menu-item>
        </el-menu>
      </el-aside>

      <el-container>
        <el-header style="background: #fff; border-bottom: 1px solid #eee; display: flex; align-items: center; justify-content: space-between;">
          <h2 style="margin: 0; font-size: 18px; color: #333;">SIP3 Server Management</h2>
          <div style="display: flex; align-items: center; gap: 12px;">
            <el-icon><UserFilled /></el-icon>
            <span style="color: #555; font-size: 14px;">{{ authStore.username }}</span>
            <el-button size="small" type="danger" plain @click="handleLogout">退出登录</el-button>
          </div>
        </el-header>
        <el-main style="background: #f0f2f5;">
          <router-view />
        </el-main>
      </el-container>
    </el-container>
  </template>

  <!-- Unauthenticated: just render the router view (Login page) -->
  <template v-else>
    <router-view />
  </template>
</template>

<script setup>
import { useRouter } from 'vue-router'
import { ElMessage } from 'element-plus'
import { useAuthStore } from './store/auth'

const router = useRouter()
const authStore = useAuthStore()

const handleLogout = () => {
  authStore.logout()
  ElMessage.success('已退出登录')
  router.push('/login')
}
</script>

<style>
* { box-sizing: border-box; }
body { margin: 0; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; }
.logo {
  height: 64px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 20px;
  font-weight: bold;
  border-bottom: 1px solid rgba(255,255,255,0.1);
}
</style>
