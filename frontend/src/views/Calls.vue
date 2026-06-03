<template>
  <div class="sip-page">
    <PageHeader title="通话记录" subtitle="CDR 列表、过滤与 CSV 导出">
      <template #actions>
        <el-button :icon="Download" @click="exportCsv" :loading="exporting">
          导出 CSV
        </el-button>
        <el-button :icon="Refresh" @click="load(1)">刷新</el-button>
      </template>
    </PageHeader>

    <el-card class="sip-filter-card">
      <el-form :inline="true" :model="filters" label-position="top">
        <el-form-item label="主叫/被叫">
          <el-input
            v-model="filters.party"
            placeholder="子串匹配"
            clearable
            style="width: 200px"
          />
        </el-form-item>
        <el-form-item label="状态">
          <el-select v-model="filters.status" placeholder="全部" clearable style="width: 140px">
            <el-option label="trying" value="trying" />
            <el-option label="answered" value="answered" />
            <el-option label="ended" value="ended" />
            <el-option label="failed" value="failed" />
          </el-select>
        </el-form-item>
        <el-form-item label="起始时间">
          <el-date-picker
            v-model="filters.since"
            type="datetime"
            placeholder="不限"
            value-format="YYYY-MM-DD HH:mm:ss"
            style="width: 220px"
          />
        </el-form-item>
        <el-form-item>
          <el-button type="primary" @click="load(1)">查询</el-button>
          <el-button @click="resetFilters">重置</el-button>
        </el-form-item>
      </el-form>
    </el-card>

    <el-card>
      <el-table
        :data="rows"
        stripe
        v-loading="loading"
        empty-text="暂无记录"
        style="width: 100%"
      >
        <el-table-column label="Call-ID" min-width="200">
          <template #default="{ row }">
            <code class="addr" :title="row.call_id">
              {{ truncate(row.call_id, 24) }}
              <el-button
                v-if="row.call_id"
                size="small"
                link
                @click="copyText(row.call_id)"
              >复制</el-button>
            </code>
          </template>
        </el-table-column>
        <el-table-column prop="caller" label="主叫" min-width="120" />
        <el-table-column prop="callee" label="被叫" min-width="120" />
        <el-table-column label="状态" width="100">
          <template #default="{ row }">
            <StatusTag :status="row.status" />
          </template>
        </el-table-column>
        <el-table-column label="开始时间" min-width="160">
          <template #default="{ row }">
            {{ formatDate(row.started_at) }}
          </template>
        </el-table-column>
        <el-table-column label="时长" width="90">
          <template #default="{ row }">
            {{ formatDuration(row.duration_secs) }}
          </template>
        </el-table-column>
        <el-table-column label="挂断原因" min-width="100">
          <template #default="{ row }">
            {{ row.hangup_cause || '—' }}
          </template>
        </el-table-column>
        <el-table-column label="SIP 状态码" width="100">
          <template #default="{ row }">
            {{ row.sip_response_code || '—' }}
          </template>
        </el-table-column>
        <el-table-column label="录音" min-width="100">
          <template #default="{ row }">
            <span v-if="row.recording_key">{{ truncate(row.recording_key, 18) }}</span>
            <span v-else>—</span>
          </template>
        </el-table-column>
      </el-table>

      <div class="sip-pagination">
        <el-pagination
          v-model:current-page="page"
          v-model:page-size="pageSize"
          :page-sizes="[20, 50, 100, 200]"
          :total="total"
          layout="total, sizes, prev, pager, next, jumper"
          @current-change="load()"
          @size-change="load(1)"
        />
      </div>
    </el-card>
  </div>
</template>

<script setup>
import { ref, reactive, onMounted } from 'vue'
import { ElMessage } from 'element-plus'
import { Download, Refresh } from '@element-plus/icons-vue'
import PageHeader from '../components/PageHeader.vue'
import StatusTag from '../components/StatusTag.vue'
import api from '../utils/api'

const rows = ref([])
const total = ref(0)
const page = ref(1)
const pageSize = ref(50)
const loading = ref(false)
const exporting = ref(false)
const filters = reactive({ party: '', status: '', since: '' })

function truncate(text, n) {
  if (!text) return ''
  return text.length > n ? text.slice(0, n) + '…' : text
}

function copyText(text) {
  navigator.clipboard?.writeText(text).then(
    () => ElMessage.success('已复制'),
    () => ElMessage.error('复制失败')
  )
}

function formatDate(v) {
  if (!v) return '—'
  const d = new Date(v)
  if (Number.isNaN(d.getTime())) return v
  return d.toLocaleString()
}

function formatDuration(secs) {
  if (secs == null) return '—'
  if (secs < 60) return `${secs}s`
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return `${m}m${s.toString().padStart(2, '0')}s`
}

async function load(targetPage) {
  if (targetPage) page.value = targetPage
  loading.value = true
  try {
    const params = {
      limit: pageSize.value,
      offset: (page.value - 1) * pageSize.value,
    }
    if (filters.status) params.status = filters.status
    if (filters.since) params.since = filters.since
    if (filters.party) {
      // server uses caller/callee LIKE; we send both with same substring
      params.caller = filters.party
      params.callee = filters.party
    }
    const { data } = await api.get('/calls', { params })
    rows.value = data
    // Use a second query for total count is heavy; approximate from row count
    total.value = data.length < pageSize.value
      ? (page.value - 1) * pageSize.value + data.length
      : page.value * pageSize.value + 1
  } catch (e) {
    ElMessage.error('加载失败：' + (e.response?.data || e.message))
  } finally {
    loading.value = false
  }
}

function resetFilters() {
  filters.party = ''
  filters.status = ''
  filters.since = ''
  load(1)
}

async function exportCsv() {
  exporting.value = true
  try {
    const params = {}
    if (filters.status) params.status = filters.status
    if (filters.since) params.since = filters.since
    if (filters.party) {
      params.caller = filters.party
      params.callee = filters.party
    }
    const res = await api.get('/calls', { params: { ...params, format: 'csv' }, responseType: 'blob' })
    const url = URL.createObjectURL(res.data)
    const link = document.createElement('a')
    link.href = url
    link.download = `sip3-cdr-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')}.csv`
    document.body.appendChild(link)
    link.click()
    document.body.removeChild(link)
    URL.revokeObjectURL(url)
    ElMessage.success('导出已触发')
  } catch (e) {
    ElMessage.error('导出失败：' + (e.response?.data || e.message))
  } finally {
    exporting.value = false
  }
}

onMounted(() => load(1))
</script>

<style scoped>
.sip-filter-card { margin-bottom: 16px; }
.sip-pagination { margin-top: 16px; display: flex; justify-content: flex-end; }
code.addr {
  font-family: ui-monospace, SFMono-Regular, monospace;
  font-size: 12px;
  color: var(--el-text-color-regular);
}
</style>
