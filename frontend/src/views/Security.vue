<template>
  <div class="sip-page">
    <PageHeader title="安全监控" subtitle="认证失败、自动封禁与最近安全事件">
      <template #actions>
        <el-button :icon="Refresh" @click="refreshAll" :loading="loading">刷新</el-button>
      </template>
    </PageHeader>

    <div class="sip-stat-grid kpi-grid">
      <StatCard :icon="WarningFilled" tone="warning" label="近 24h 认证失败"
                :value="summary.auth_failed_24h ?? 0" />
      <StatCard :icon="Lock"          tone="danger"  label="近 24h 自动封禁"
                :value="summary.blocked_24h ?? 0" />
      <StatCard :icon="CircleClose"   tone="danger"  label="当前自动封禁"
                :value="summary.active_auto_blocks ?? 0" />
    </div>

    <el-card style="margin-top: 16px; margin-bottom: 16px;">
      <template #header>
        <div class="card-head">
          <span>当前自动封禁</span>
          <el-input v-model="cidrFilter" placeholder="CIDR 过滤" clearable size="small" style="width: 180px;"
            :prefix-icon="Search" />
        </div>
      </template>
      <el-table :data="filteredBlocks" v-loading="loading">
        <el-table-column prop="cidr" label="CIDR" width="220">
          <template #default="{ row }"><code class="addr">{{ row.cidr }}</code></template>
        </el-table-column>
        <el-table-column prop="description" label="原因" show-overflow-tooltip />
        <el-table-column prop="priority" label="优先级" width="90" align="center" />
        <el-table-column prop="created_at" label="封禁时间" width="180">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.created_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="110" fixed="right">
          <template #default="{ row }">
            <el-button text type="primary" @click="handleUnblock(row)">解封</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <EmptyState title="暂无封禁" subtitle="目前没有自动封禁的 IP" :icon="CircleCheck" />
        </template>
      </el-table>
    </el-card>

    <el-card>
      <template #header>
        <div class="card-head">
          <span>安全事件（最近 200 条）</span>
          <el-input v-model="ipFilter" placeholder="来源 IP 过滤" clearable size="small" style="width: 180px;"
            :prefix-icon="Search" />
        </div>
      </template>
      <el-table :data="filteredEvents" v-loading="loading" :max-height="520">
        <el-table-column prop="created_at" label="时间" width="180">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.created_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="surface" label="入口" width="100">
          <template #default="{ row }">
            <el-tag size="small" effect="plain">{{ row.surface }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="event_type" label="事件" width="140">
          <template #default="{ row }">
            <span class="event-type" :class="eventClass(row.event_type)">{{ row.event_type }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="source_ip" label="来源IP" width="150">
          <template #default="{ row }"><code class="addr">{{ row.source_ip }}</code></template>
        </el-table-column>
        <el-table-column prop="username" label="用户名" width="120" />
        <el-table-column prop="detail" label="详情" show-overflow-tooltip />
        <template #empty>
          <EmptyState title="暂无事件" />
        </template>
      </el-table>
    </el-card>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Refresh, Search, WarningFilled, Lock, CircleClose, CircleCheck } from '@element-plus/icons-vue'
import api from '../utils/api'
import PageHeader from '../components/PageHeader.vue'
import StatCard from '../components/StatCard.vue'
import EmptyState from '../components/EmptyState.vue'

const loading = ref(false)
const summary = ref({})
const blocks = ref([])
const events = ref([])
const ipFilter = ref('')
const cidrFilter = ref('')

const formatDate = (d) => d ? new Date(d).toLocaleString() : '-'

const filteredEvents = computed(() => {
  const q = ipFilter.value.trim()
  if (!q) return events.value
  return events.value.filter(e => (e.source_ip || '').includes(q))
})

const filteredBlocks = computed(() => {
  const q = cidrFilter.value.trim()
  if (!q) return blocks.value
  return blocks.value.filter(b => (b.cidr || '').includes(q))
})

const eventClass = (t) => {
  const s = (t || '').toLowerCase()
  if (s.includes('fail') || s.includes('block') || s.includes('deny') || s.includes('reject')) return 'event-danger'
  if (s.includes('warn')) return 'event-warning'
  return 'event-info'
}

const refreshAll = async () => {
  loading.value = true
  try {
    const [s, b, e] = await Promise.all([
      api.get('/security/summary'),
      api.get('/security/blocks'),
      api.get('/security/events', { params: { limit: 200 } }),
    ])
    summary.value = s.data || {}
    blocks.value = b.data?.data || []
    events.value = e.data?.data || []
  } catch (err) {
    ElMessage.error(err.response?.data || err.message || '加载安全数据失败')
  } finally {
    loading.value = false
  }
}

const handleUnblock = async (row) => {
  await ElMessageBox.confirm(`确认解封 ${row.cidr} ?`, '解封确认', { type: 'warning' })
  try {
    await api.post('/security/blocks/unblock', { cidr: row.cidr })
    ElMessage.success('解封成功')
    await refreshAll()
  } catch (err) {
    ElMessage.error(err.response?.data || err.message || '解封失败')
  }
}

onMounted(refreshAll)
</script>

<style scoped>
.kpi-grid { grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); }
.card-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  width: 100%;
  flex-wrap: wrap;
  gap: 8px;
}
.addr {
  font-family: 'SF Mono', 'Cascadia Code', Consolas, monospace;
  font-size: 12px;
  color: var(--sip-text-2);
}
.text-secondary { color: var(--sip-text-2); font-size: 13px; }
.event-type {
  display: inline-block;
  padding: 2px 8px;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 500;
}
.event-danger  { background: var(--sip-danger-soft);  color: var(--sip-danger); }
.event-warning { background: var(--sip-warning-soft); color: var(--sip-warning); }
.event-info    { background: var(--sip-primary-soft); color: var(--sip-primary); }
</style>
