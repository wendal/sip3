<template>
  <button class="theme-toggle" :title="theme === 'dark' ? '切换浅色' : '切换深色'" @click="toggle">
    <el-icon v-if="theme === 'dark'"><Sunny /></el-icon>
    <el-icon v-else><Moon /></el-icon>
  </button>
</template>

<script setup>
import { useTheme } from '../composables/useTheme'
import { Sunny, Moon } from '@element-plus/icons-vue'

const { theme, toggle } = useTheme()
</script>

<style scoped>
.theme-toggle {
  width: 36px;
  height: 36px;
  border-radius: 50%;
  border: 1px solid var(--sip-border);
  background: var(--sip-surface);
  color: var(--sip-text-2);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: background 0.15s, color 0.15s, transform 0.1s;
  font-size: 16px;
}
.theme-toggle:hover {
  background: var(--sip-surface-2);
  color: var(--sip-text);
}
.theme-toggle:active { transform: scale(0.94); }
</style>
