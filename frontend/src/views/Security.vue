<template>
  <div>
    <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:16px;">
      <h2 style="margin:0;">安全监控</h2>
      <el-button @click="refreshAll" :loading="loading">
        <el-icon><Refresh /></el-icon> 刷新
      </el-button>
    </div>

    <el-row :gutter="12" style="margin-bottom:16px;">
      <el-col :span="8">
        <el-card>
          <div style="font-size:12px;color:#999;">近 24h 认证失败</div>
          <div style="font-size:28px;font-weight:600;">{{ summary.auth_failed_24h ?? 0 }}</div>
        </el-card>
      </el-col>
      <el-col :span="8">
        <el-card>
          <div style="font-size:12px;color:#999;">近 24h 自动封禁</div>
          <div style="font-size:28px;font-weight:600;">{{ summary.blocked_24h ?? 0 }}</div>
        </el-card>
      </el-col>
      <el-col :span="8">
        <el-card>
          <div style="font-size:12px;color:#999;">当前自动封禁</div>
          <div style="font-size:28px;font-weight:600;">{{ summary.active_auto_blocks ?? 0 }}</div>
        </el-card>
      </el-col>
    </el-row>

    <el-card style="margin-bottom:16px;">
      <template #header>
        <div style="display:flex; justify-content:space-between; align-items:center;">
          <span>当前自动封禁</span>
          <el-input v-model="cidrFilter" placeholder="CIDR 过滤" clearable size="small" style="width:180px;" />
        </div>
      </template>
      <el-table :data="filteredBlocks" stripe v-loading="loading">
        <el-table-column prop="cidr" label="CIDR" width="220" />
        <el-table-column prop="description" label="原因" />
        <el-table-column prop="priority" label="优先级" width="90" />
        <el-table-column prop="created_at" label="封禁时间" width="180">
          <template #default="{ row }">{{ formatDate(row.created_at) }}</template>
        </el-table-column>
        <el-table-column label="操作" width="110">
          <template #default="{ row }">
            <el-button size="small" type="danger" plain @click="handleUnblock(row)">解封</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <el-card>
      <template #header>
        <div style="display:flex; justify-content:space-between; align-items:center;">
          <span>安全事件（最近 200 条）</span>
          <el-input v-model="ipFilter" placeholder="来源 IP 过滤" clearable size="small" style="width:180px;" />
        </div>
      </template>
      <el-table :data="filteredEvents" stripe v-loading="loading">
        <el-table-column prop="created_at" label="时间" width="180">
          <template #default="{ row }">{{ formatDate(row.created_at) }}</template>
        </el-table-column>
        <el-table-column prop="surface" label="入口" width="120" />
        <el-table-column prop="event_type" label="事件" width="120" />
        <el-table-column prop="source_ip" label="来源IP" width="150" />
        <el-table-column prop="username" label="用户名" width="120" />
        <el-table-column prop="detail" label="详情" show-overflow-tooltip />
      </el-table>
    </el-card>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import api from '../utils/api'

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
