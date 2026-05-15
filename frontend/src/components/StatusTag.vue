<template>
  <span class="status-tag" :class="`status-tag--${kind}`">
    <span class="status-tag__dot" />
    {{ label }}
  </span>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  status: { type: String, default: '' },
  /** Optional override for the visible label */
  label: { type: String, default: '' },
})

// Map domain status keywords -> visual kind
const STATUS_MAP = {
  // call statuses
  answered: { kind: 'success', label: '接通' },
  ended:    { kind: 'neutral', label: '结束' },
  cancelled:{ kind: 'warning', label: '取消' },
  trying:   { kind: 'info',    label: '呼叫中' },
  failed:   { kind: 'danger',  label: '失败' },
  // generic
  enabled:  { kind: 'success', label: '启用' },
  disabled: { kind: 'neutral', label: '禁用' },
  allow:    { kind: 'success', label: 'Allow' },
  deny:     { kind: 'danger',  label: 'Deny' },
  delivered:{ kind: 'success', label: '已送达' },
  sending:  { kind: 'info',    label: '发送中' },
}

const resolved = computed(() => STATUS_MAP[props.status] || { kind: 'neutral', label: props.label || props.status })
const kind = computed(() => resolved.value.kind)
const label = computed(() => props.label || resolved.value.label)
</script>

<style scoped>
.status-tag {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px 3px 8px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 500;
  line-height: 1.4;
  background: var(--sip-surface-2);
  color: var(--sip-text-2);
  white-space: nowrap;
}
.status-tag__dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}
.status-tag--success { background: var(--sip-success-soft); color: var(--sip-success); }
.status-tag--danger  { background: var(--sip-danger-soft);  color: var(--sip-danger); }
.status-tag--warning { background: var(--sip-warning-soft); color: var(--sip-warning); }
.status-tag--info    { background: var(--sip-primary-soft); color: var(--sip-primary); }
.status-tag--neutral { background: var(--sip-surface-2); color: var(--sip-text-2); }
</style>
