<template>
  <div class="empty-state">
    <div class="empty-state__icon">
      <el-icon><component :is="icon || DocumentRemove" /></el-icon>
    </div>
    <div class="empty-state__title">{{ title }}</div>
    <div v-if="subtitle" class="empty-state__subtitle">{{ subtitle }}</div>
    <div v-if="$slots.default" class="empty-state__actions"><slot /></div>
  </div>
</template>

<script setup>
import { DocumentRemove } from '@element-plus/icons-vue'
defineProps({
  title: { type: String, default: '暂无数据' },
  subtitle: { type: String, default: '' },
  icon: { type: [Object, Function], default: null },
})
</script>

<style scoped>
.empty-state {
  text-align: center;
  padding: 40px 16px;
  color: var(--sip-text-2);
}
.empty-state__icon {
  width: 56px;
  height: 56px;
  margin: 0 auto 12px;
  border-radius: 50%;
  background: var(--sip-surface-2);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 24px;
  color: var(--sip-text-3);
}
.empty-state__title { font-size: 14px; color: var(--sip-text); font-weight: 500; }
.empty-state__subtitle { font-size: 12px; color: var(--sip-text-2); margin-top: 4px; }
.empty-state__actions { margin-top: 14px; }
</style>
