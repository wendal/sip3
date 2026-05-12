<template>
  <div>
    <h2 style="margin-bottom: 20px;">System Status</h2>

    <el-card style="margin-bottom: 20px;">
      <template #header>
        <div style="display: flex; justify-content: space-between;">
          <span>Active Registrations ({{ store.registrations.length }})</span>
          <el-button text @click="store.fetchRegistrations()">
            <el-icon><Refresh /></el-icon> Refresh
          </el-button>
        </div>
      </template>
      <el-table :data="store.registrations" stripe>
        <el-table-column prop="username" label="Username" width="150" />
        <el-table-column prop="domain" label="Domain" width="200" />
        <el-table-column prop="contact_uri" label="Contact URI" show-overflow-tooltip />
        <el-table-column prop="source_ip" label="Source IP" width="140" />
        <el-table-column prop="source_port" label="Port" width="80" />
        <el-table-column prop="user_agent" label="User Agent" show-overflow-tooltip />
        <el-table-column prop="expires_at" label="Expires At" width="160">
          <template #default="{ row }">{{ formatDate(row.expires_at) }}</template>
        </el-table-column>
      </el-table>
    </el-card>

    <el-card>
      <template #header>
        <div style="display: flex; justify-content: space-between;">
          <span>Call Records ({{ store.calls.length }})</span>
          <el-button text @click="store.fetchCalls()">
            <el-icon><Refresh /></el-icon> Refresh
          </el-button>
        </div>
      </template>
      <el-table :data="store.calls" stripe>
        <el-table-column prop="call_id" label="Call-ID" show-overflow-tooltip width="220" />
        <el-table-column prop="caller" label="Caller" show-overflow-tooltip />
        <el-table-column prop="callee" label="Callee" show-overflow-tooltip />
        <el-table-column prop="status" label="Status" width="110">
          <template #default="{ row }">
            <el-tag :type="statusColor(row.status)" size="small">{{ row.status }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="started_at" label="Started" width="160">
          <template #default="{ row }">{{ formatDate(row.started_at) }}</template>
        </el-table-column>
        <el-table-column prop="ended_at" label="Ended" width="160">
          <template #default="{ row }">{{ formatDate(row.ended_at) }}</template>
        </el-table-column>
        <el-table-column label="Duration" width="100">
          <template #default="{ row }">{{ duration(row) }}</template>
        </el-table-column>
      </el-table>
    </el-card>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted } from 'vue'
import { useSipStore } from '../store'

const store = useSipStore()

const formatDate = (d) => d ? new Date(d).toLocaleString() : '-'

const statusColor = (status) => {
  const map = { answered: 'success', ended: 'info', cancelled: 'warning', trying: '', failed: 'danger' }
  return map[status] || ''
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

const AUTO_REFRESH_INTERVAL_MS = 15_000 // 15 seconds

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
