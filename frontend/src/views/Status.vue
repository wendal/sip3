<template>
  <div class="sip-page">
    <PageHeader title="系统状态" subtitle="实时注册与通话记录，每 15 秒自动刷新" />

    <el-card style="margin-bottom: 16px;">
      <template #header>
        <div class="card-head">
          <span>当前注册 ({{ store.registrations.length }})</span>
          <el-button text :icon="Refresh" @click="store.fetchRegistrations()">刷新</el-button>
        </div>
      </template>
      <el-table :data="store.registrations">
        <el-table-column prop="username" label="用户名" width="120" />
        <el-table-column prop="domain" label="域" width="180" show-overflow-tooltip />
        <el-table-column prop="contact_uri" label="联系地址" show-overflow-tooltip />
        <el-table-column label="来源" width="180">
          <template #default="{ row }">
            <code class="addr">{{ row.source_ip }}:{{ row.source_port }}</code>
          </template>
        </el-table-column>
        <el-table-column prop="user_agent" label="用户代理" show-overflow-tooltip />
        <el-table-column prop="expires_at" label="到期时间" width="160">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.expires_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="120" fixed="right">
          <template #default="{ row }">
            <el-button text type="danger" @click="handleDeregister(row)">强制注销</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <EmptyState title="暂无注册" subtitle="尚无 SIP 客户端注册" />
        </template>
      </el-table>
    </el-card>

    <el-card>
      <template #header>
        <div class="card-head card-head--filter">
          <span>通话记录 ({{ filteredCalls.length }})</span>
          <div class="filter-group">
            <el-input
              v-model="callSearch"
              placeholder="主叫/被叫…"
              style="width: 180px;"
              clearable
              size="small"
              :prefix-icon="Search"
            />
            <el-select v-model="callStatusFilter" placeholder="全部状态" clearable size="small" style="width: 130px;">
              <el-option label="全部" value="" />
              <el-option label="接通" value="answered" />
              <el-option label="结束" value="ended" />
              <el-option label="取消" value="cancelled" />
              <el-option label="呼叫中" value="trying" />
              <el-option label="失败" value="failed" />
            </el-select>
            <el-button text :icon="Refresh" @click="store.fetchCalls()" />
          </div>
        </div>
      </template>
      <el-table :data="pagedCalls">
        <el-table-column prop="call_id" label="Call-ID" show-overflow-tooltip width="200">
          <template #default="{ row }"><code class="addr">{{ row.call_id }}</code></template>
        </el-table-column>
        <el-table-column prop="caller" label="主叫" show-overflow-tooltip />
        <el-table-column prop="callee" label="被叫" show-overflow-tooltip />
        <el-table-column label="状态" width="100">
          <template #default="{ row }">
            <StatusTag :status="row.status" />
          </template>
        </el-table-column>
        <el-table-column prop="started_at" label="开始时间" width="160">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.started_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="ended_at" label="结束时间" width="160">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.ended_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="时长" width="90" align="right">
          <template #default="{ row }">
            <span class="num">{{ duration(row) }}</span>
          </template>
        </el-table-column>
        <template #empty>
          <EmptyState title="暂无通话记录" />
        </template>
      </el-table>
      <div style="margin-top: 12px; display: flex; justify-content: flex-end;">
        <el-pagination
          v-model:current-page="currentPage"
          :page-size="pageSize"
          :total="filteredCalls.length"
          layout="total, prev, pager, next"
          small
          background
        />
      </div>
    </el-card>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Refresh, Search } from '@element-plus/icons-vue'
import { useSipStore } from '../store'
import api from '../utils/api'
import PageHeader from '../components/PageHeader.vue'
import StatusTag from '../components/StatusTag.vue'
import EmptyState from '../components/EmptyState.vue'

const store = useSipStore()
const callSearch = ref('')
const callStatusFilter = ref('')
const currentPage = ref(1)
const pageSize = 20

const formatDate = (d) => d ? new Date(d).toLocaleString() : '-'

const duration = (row) => {
  if (!row.answered_at) return '-'
  const end = row.ended_at ? new Date(row.ended_at) : new Date()
  const start = new Date(row.answered_at)
  const secs = Math.floor((end - start) / 1000)
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return `${m}:${String(s).padStart(2, '0')}`
}

const filteredCalls = computed(() => {
  let list = store.calls
  if (callStatusFilter.value) {
    list = list.filter(c => c.status === callStatusFilter.value)
  }
  if (callSearch.value.trim()) {
    const q = callSearch.value.trim().toLowerCase()
    list = list.filter(c =>
      c.caller.toLowerCase().includes(q) || c.callee.toLowerCase().includes(q)
    )
  }
  return list
})

const pagedCalls = computed(() => {
  const start = (currentPage.value - 1) * pageSize
  return filteredCalls.value.slice(start, start + pageSize)
})

const handleDeregister = async (row) => {
  await ElMessageBox.confirm(
    `确认强制注销 "${row.username}@${row.domain}"？`,
    '强制注销',
    { type: 'warning' }
  )
  try {
    await api.delete(`/registrations/${row.id}`)
    ElMessage.success('已强制注销')
    await store.fetchRegistrations()
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '操作失败')
  }
}

const AUTO_REFRESH_INTERVAL_MS = 15_000

let timer
onMounted(async () => {
  await Promise.all([store.fetchRegistrations(), store.fetchCalls()])
  timer = setInterval(() => {
    store.fetchRegistrations()
    store.fetchCalls()
  }, AUTO_REFRESH_INTERVAL_MS)
})
onUnmounted(() => clearInterval(timer))
</script>

<style scoped>
.card-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  width: 100%;
}
.card-head--filter { flex-wrap: wrap; gap: 8px; }
.filter-group { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
.addr {
  font-family: 'SF Mono', 'Cascadia Code', Consolas, monospace;
  font-size: 12px;
  color: var(--sip-text-2);
}
.text-secondary { color: var(--sip-text-2); font-size: 13px; }
</style>
