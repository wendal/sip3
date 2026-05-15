<template>
  <div class="stat-card" :class="`stat-card--${tone}`">
    <div class="stat-card__icon">
      <el-icon><component :is="icon" /></el-icon>
    </div>
    <div class="stat-card__body">
      <div class="stat-card__label">{{ label }}</div>
      <div class="stat-card__value num">{{ value }}</div>
      <div v-if="hint" class="stat-card__hint">{{ hint }}</div>
    </div>
  </div>
</template>

<script setup>
defineProps({
  label: { type: String, required: true },
  value: { type: [String, Number], required: true },
  icon:  { type: [Object, Function], required: true },
  tone:  { type: String, default: 'primary' }, // primary | success | warning | danger | info
  hint:  { type: String, default: '' },
})
</script>

<style scoped>
.stat-card {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 18px 20px;
  border-radius: var(--sip-radius);
  background: var(--sip-surface);
  border: 1px solid var(--sip-border);
  box-shadow: var(--sip-shadow-sm);
  transition: transform 0.18s ease, box-shadow 0.18s ease;
  position: relative;
  overflow: hidden;
}
.stat-card::before {
  content: '';
  position: absolute;
  left: 0; top: 0; bottom: 0;
  width: 4px;
  background: var(--accent, var(--sip-primary));
}
.stat-card:hover {
  transform: translateY(-2px);
  box-shadow: var(--sip-shadow);
}
.stat-card__icon {
  width: 44px;
  height: 44px;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 22px;
  background: var(--accent-soft, var(--sip-primary-soft));
  color: var(--accent, var(--sip-primary));
  flex-shrink: 0;
}
.stat-card__body { min-width: 0; flex: 1; }
.stat-card__label {
  font-size: 12px;
  color: var(--sip-text-2);
  font-weight: 500;
  letter-spacing: 0.3px;
}
.stat-card__value {
  font-size: 26px;
  font-weight: 700;
  color: var(--sip-text);
  margin-top: 4px;
  line-height: 1.1;
  letter-spacing: -0.5px;
}
.stat-card__hint {
  font-size: 11px;
  color: var(--sip-text-3);
  margin-top: 4px;
}

.stat-card--primary { --accent: var(--sip-primary); --accent-soft: var(--sip-primary-soft); }
.stat-card--success { --accent: var(--sip-success); --accent-soft: var(--sip-success-soft); }
.stat-card--warning { --accent: var(--sip-warning); --accent-soft: var(--sip-warning-soft); }
.stat-card--danger  { --accent: var(--sip-danger);  --accent-soft: var(--sip-danger-soft); }
.stat-card--info    { --accent: var(--sip-info);    --accent-soft: rgba(90, 200, 250, 0.15); }
</style>
