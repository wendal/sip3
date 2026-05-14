<template>
  <div>
    <h2 style="margin-bottom: 20px;">系统状态</h2>

    <!-- 当前注册 -->
    <el-card style="margin-bottom: 20px;">
      <template #header>
        <div style="display: flex; justify-content: space-between; align-items: center;">
          <span>当前注册 ({{ store.registrations.length }})</span>
          <el-button text @click="store.fetchRegistrations()">
            <el-icon><Refresh /></el-icon> 刷新
          </el-button>
        </div>
      </template>
      <el-table :data="store.registrations" stripe>
        <el-table-column prop="username" label="用户名" width="130" />
        <el-table-column prop="domain" label="域" width="180" />
        <el-table-column prop="contact_uri" label="联系地址" show-overflow-tooltip />
        <el-table-column prop="source_ip" label="来源IP" width="130" />
        <el-table-column prop="source_port" label="端口" width="70" />
        <el-table-column prop="user_agent" label="用户代理" show-overflow-tooltip />
        <el-table-column prop="expires_at" label="到期时间" width="160">
          <template #default="{ row }">{{ formatDate(row.expires_at) }}</template>
        </el-table-column>
        <el-table-column label="操作" width="100">
          <template #default="{ row }">
            <el-button size="small" type="danger" plain @click="handleDeregister(row)">强制注销</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 通话记录 -->
    <el-card>
      <template #header>
        <div style="display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px;">
          <span>通话记录 ({{ filteredCalls.length }})</span>
          <div style="display: flex; gap: 8px; align-items: center;">
            <el-input
              v-model="callSearch"
              placeholder="主叫/被叫搜索..."
              style="width: 180px;"
              clearable
              size="small"
            />
            <el-select v-model="callStatusFilter" placeholder="全部状态" clearable size="small" style="width: 110px;">
              <el-option label="全部" value="" />
              <el-option label="接通" value="answered" />
              <el-option label="结束" value="ended" />
              <el-option label="取消" value="cancelled" />
              <el-option label="呼叫中" value="trying" />
              <el-option label="失败" value="failed" />
            </el-select>
            <el-button text @click="store.fetchCalls()">
              <el-icon><Refresh /></el-icon>
            </el-button>
          </div>
        </div>
      </template>
      <el-table :data="pagedCalls" stripe>
        <el-table-column prop="call_id" label="Call-ID" show-overflow-tooltip width="200" />
        <el-table-column prop="caller" label="主叫" show-overflow-tooltip />
        <el-table-column prop="callee" label="被叫" show-overflow-tooltip />
        <el-table-column prop="status" label="状态" width="100">
          <template #default="{ row }">
            <el-tag :type="statusColor(row.status)" size="small">{{ statusLabel(row.status) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="started_at" label="开始时间" width="160">
          <template #default="{ row }">{{ formatDate(row.started_at) }}</template>
        </el-table-column>
        <el-table-column prop="ended_at" label="结束时间" width="160">
          <template #default="{ row }">{{ formatDate(row.ended_at) }}</template>
        </el-table-column>
        <el-table-column label="时长" width="90">
          <template #default="{ row }">{{ duration(row) }}</template>
        </el-table-column>
      </el-table>
      <div style="margin-top: 12px; display: flex; justify-content: flex-end;">
        <el-pagination
          v-model:current-page="currentPage"
          :page-size="pageSize"
          :total="filteredCalls.length"
          layout="total, prev, pager, next"
          small
        />
      </div>
    </el-card>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useSipStore } from '../store'
import api from '../utils/api'

const store = useSipStore()
const callSearch = ref('')
const callStatusFilter = ref('')
const currentPage = ref(1)
const pageSize = 20

const formatDate = (d) => d ? new Date(d).toLocaleString() : '-'

const statusColor = (status) => {
  const map = { answered: 'success', ended: 'info', cancelled: 'warning', trying: '', failed: 'danger' }
  return map[status] || ''
}

const statusLabel = (status) => {
  const map = { answered: '接通', ended: '结束', cancelled: '取消', trying: '呼叫中', failed: '失败' }
  return map[status] || status
}

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
