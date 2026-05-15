<template>
  <div class="sip-page-header">
    <div>
      <h2 class="sip-page-header__title">
        <el-icon v-if="icon" class="sip-page-header__icon"><component :is="icon" /></el-icon>
        {{ title }}
      </h2>
      <div v-if="subtitle" class="sip-page-header__subtitle">{{ subtitle }}</div>
    </div>
    <div v-if="$slots.actions" class="sip-page-header__actions">
      <slot name="actions" />
    </div>
  </div>
</template>

<script setup>
defineProps({
  title: { type: String, required: true },
  subtitle: { type: String, default: '' },
  icon: { type: [String, Object, Function], default: null },
})
</script>

<style scoped>
.sip-page-header__icon {
  margin-right: 8px;
  color: var(--sip-primary);
  vertical-align: -2px;
}
</style>
