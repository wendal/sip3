<template>
  <div class="sip-page">
    <PageHeader
      title="语音信箱"
      subtitle="管理 SIP 账号语音信箱与留言状态"
    >
      <template #actions>
        <el-button type="primary" :icon="Plus" @click="openCreate">添加信箱</el-button>
      </template>
    </PageHeader>

    <el-card>
      <el-table :data="boxes" v-loading="loadingBoxes">
        <el-table-column prop="username" label="用户名" min-width="140">
          <template #default="{ row }">
            <code>{{ row.username }}</code>
            <div class="mailbox-hint">sip:{{ row.username }}@{{ row.domain }}</div>
          </template>
        </el-table-column>
        <el-table-column prop="domain" label="域" min-width="160" show-overflow-tooltip />
        <el-table-column prop="enabled" label="状态" width="100">
          <template #default="{ row }">
            <el-switch
              :model-value="!!row.enabled"
              @change="toggleEnabled(row)"
              inline-prompt active-text="启用" inactive-text="禁用"
            />
          </template>
        </el-table-column>
        <el-table-column prop="new_count" label="新留言" width="90" align="right" />
        <el-table-column prop="saved_count" label="已保存" width="90" align="right" />
        <el-table-column prop="no_answer_secs" label="无应答秒数" width="115" align="right" />
        <el-table-column prop="max_message_secs" label="最长留言秒数" width="125" align="right" />
        <el-table-column prop="max_messages" label="容量上限" width="100" align="right" />
        <el-table-column label="操作" width="220" fixed="right">
          <template #default="{ row }">
            <el-button text type="primary" @click="fetchMessages(row)">留言</el-button>
            <el-button text @click="openEdit(row)">编辑</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <EmptyState title="暂无语音信箱" subtitle="点击右上角“添加信箱”为账号开启语音信箱" :icon="MessageBox" />
        </template>
      </el-table>
    </el-card>

    <el-drawer
      v-model="messagesVisible"
      :title="`留言 — ${selectedBox?.username || ''}@${selectedBox?.domain || ''}`"
      size="720px"
      direction="rtl"
    >
      <el-table :data="messages" v-loading="loadingMessages">
        <el-table-column prop="caller" label="主叫" min-width="150" show-overflow-tooltip />
        <el-table-column prop="duration_secs" label="时长(秒)" width="90" align="right" />
        <el-table-column prop="status" label="状态" width="90">
          <template #default="{ row }">
            <el-tag :type="statusType(row.status)" size="small">{{ statusLabel(row.status) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="created_at" label="创建时间" width="170">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.created_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="230" fixed="right">
          <template #default="{ row }">
            <el-button text type="primary" :loading="playingId === row.id" @click="playMessage(row)">播放</el-button>
            <el-button text @click="downloadMessage(row)">下载</el-button>
            <el-button
              text
              type="success"
              :disabled="row.status === 'saved'"
              @click="updateMessageStatus(row, 'saved')"
            >
              保存
            </el-button>
            <el-button
              text
              type="danger"
              :disabled="row.status === 'deleted'"
              @click="deleteMessage(row)"
            >
              删除
            </el-button>
          </template>
        </el-table-column>
        <template #empty>
          <EmptyState title="暂无留言" subtitle="新留言会显示在这里" :icon="MessageBox" />
        </template>
      </el-table>
    </el-drawer>

    <el-drawer
      v-model="drawerVisible"
      :title="editingId ? '编辑语音信箱' : '添加语音信箱'"
      size="420px"
      direction="rtl"
    >
      <el-form :model="form" label-width="110px" label-position="top">
        <el-form-item label="用户名">
          <el-input v-model="form.username" placeholder="1001" :disabled="!!editingId" />
        </el-form-item>
        <el-form-item label="域">
          <el-input v-model="form.domain" placeholder="sip.air32.cn" :disabled="!!editingId" />
        </el-form-item>
        <el-form-item label="状态">
          <el-switch
            v-model="form.enabled" :active-value="1" :inactive-value="0"
            inline-prompt active-text="启用" inactive-text="禁用"
          />
        </el-form-item>
        <el-form-item label="无应答秒数">
          <el-input-number v-model="form.no_answer_secs" :min="1" :max="600" />
        </el-form-item>
        <el-form-item label="最长留言秒数">
          <el-input-number v-model="form.max_message_secs" :min="1" :max="3600" />
        </el-form-item>
        <el-form-item label="容量上限">
          <el-input-number v-model="form.max_messages" :min="1" :max="10000" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="drawerVisible = false">取消</el-button>
        <el-button type="primary" @click="handleSubmit" :loading="submitting">
          {{ editingId ? '更新' : '创建' }}
        </el-button>
      </template>
    </el-drawer>

    <el-dialog
      v-model="audioVisible"
      :title="`播放留言 — ${audioMessage?.caller || ''}`"
      width="420px"
      @closed="clearAudioPreview"
    >
      <audio v-if="audioUrl" class="audio-player" :src="audioUrl" controls autoplay />
      <div v-else class="text-secondary">正在加载音频...</div>
      <template #footer>
        <el-button @click="audioVisible = false">关闭</el-button>
        <el-button v-if="audioMessage" type="primary" @click="downloadMessage(audioMessage)">下载</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, onMounted, onBeforeUnmount } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { MessageBox, Plus } from '@element-plus/icons-vue'
import api from '../utils/api'
import PageHeader from '../components/PageHeader.vue'
import EmptyState from '../components/EmptyState.vue'

const boxes = ref([])
const messages = ref([])
const loadingBoxes = ref(false)
const loadingMessages = ref(false)
const selectedBox = ref(null)
const drawerVisible = ref(false)
const messagesVisible = ref(false)
const editingId = ref(null)
const submitting = ref(false)
const form = ref(emptyForm())
const audioVisible = ref(false)
const audioUrl = ref('')
const audioMessage = ref(null)
const playingId = ref(null)

function emptyForm() {
  return {
    username: '',
    domain: 'sip.air32.cn',
    enabled: 1,
    no_answer_secs: 25,
    max_message_secs: 120,
    max_messages: 100,
  }
}

const formatDate = (d) => d ? new Date(d).toLocaleString() : '-'

const statusLabel = (status) => ({
  new: '新留言',
  saved: '已保存',
  deleted: '已删除',
}[status] || status || '-')

const statusType = (status) => ({
  new: 'warning',
  saved: 'success',
  deleted: 'info',
}[status] || '')

const errorMessage = (e, fallback) => {
  const data = e.response?.data
  return (typeof data === 'string' && data) || e.message || fallback
}

const fetchBoxes = async () => {
  loadingBoxes.value = true
  try {
    const res = await api.get('/voicemail/boxes')
    boxes.value = res.data.data || []
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '加载失败')
  } finally {
    loadingBoxes.value = false
  }
}

const fetchMessages = async (box) => {
  if (!box) return
  selectedBox.value = box
  messagesVisible.value = true
  loadingMessages.value = true
  try {
    const res = await api.get('/voicemail/messages', { params: { box_id: box.id } })
    messages.value = res.data.data || []
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '加载留言失败')
  } finally {
    loadingMessages.value = false
  }
}

const downloadMessage = async (row) => {
  let url = ''
  try {
    const res = await api.get(`/voicemail/messages/${row.id}/download`, { responseType: 'blob' })
    url = URL.createObjectURL(res.data)
    const link = document.createElement('a')
    link.href = url
    link.download = `voicemail-${row.id}.wav`
    document.body.appendChild(link)
    link.click()
    document.body.removeChild(link)
  } catch (e) {
    ElMessage.error(errorMessage(e, '下载失败'))
  } finally {
    if (url) {
      URL.revokeObjectURL(url)
    }
  }
}

const clearAudioPreview = () => {
  if (audioUrl.value) {
    URL.revokeObjectURL(audioUrl.value)
  }
  audioUrl.value = ''
  audioMessage.value = null
}

const playMessage = async (row) => {
  playingId.value = row.id
  try {
    clearAudioPreview()
    audioMessage.value = row
    audioVisible.value = true
    const res = await api.get(`/voicemail/messages/${row.id}/download`, { responseType: 'blob' })
    audioUrl.value = URL.createObjectURL(res.data)
  } catch (e) {
    audioVisible.value = false
    ElMessage.error(errorMessage(e, '播放失败'))
  } finally {
    playingId.value = null
  }
}

const updateMessageStatus = async (row, status) => {
  try {
    await api.put(`/voicemail/messages/${row.id}`, { status })
    ElMessage.success(status === 'saved' ? '留言已保存' : '留言已更新')
    await fetchMessages(selectedBox.value)
    await fetchBoxes()
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '操作失败')
  }
}

const deleteMessage = async (row) => {
  try {
    await ElMessageBox.confirm('确认删除该留言？删除后状态会标记为已删除。', '确认删除', { type: 'warning' })
  } catch {
    return
  }
  await updateMessageStatus(row, 'deleted')
}

const openCreate = () => {
  editingId.value = null
  form.value = emptyForm()
  drawerVisible.value = true
}

const openEdit = (row) => {
  editingId.value = row.id
  form.value = {
    username: row.username,
    domain: row.domain,
    enabled: row.enabled,
    no_answer_secs: row.no_answer_secs,
    max_message_secs: row.max_message_secs,
    max_messages: row.max_messages,
  }
  drawerVisible.value = true
}

const validateForm = () => {
  if (!editingId.value && !form.value.username?.trim()) {
    ElMessage.error('请填写用户名')
    return false
  }
  if (!form.value.domain?.trim()) {
    ElMessage.error('请填写域')
    return false
  }
  const checks = [
    ['no_answer_secs', '无应答秒数', 1, 600],
    ['max_message_secs', '最长留言秒数', 1, 3600],
    ['max_messages', '容量上限', 1, 10000],
  ]
  for (const [key, label, min, max] of checks) {
    const value = Number(form.value[key])
    if (!Number.isInteger(value) || value < min || value > max) {
      ElMessage.error(`${label}必须是 ${min}-${max} 的正整数`)
      return false
    }
    form.value[key] = value
  }
  return true
}

const handleSubmit = async () => {
  if (!validateForm()) return
  submitting.value = true
  try {
    const payload = {
      enabled: form.value.enabled,
      no_answer_secs: form.value.no_answer_secs,
      max_message_secs: form.value.max_message_secs,
      max_messages: form.value.max_messages,
    }
    if (editingId.value) {
      await api.put(`/voicemail/boxes/${editingId.value}`, payload)
      ElMessage.success('语音信箱已更新')
    } else {
      await api.post('/voicemail/boxes', {
        username: form.value.username.trim(),
        domain: form.value.domain.trim() || 'sip.air32.cn',
        ...payload,
      })
      ElMessage.success('语音信箱已创建')
    }
    drawerVisible.value = false
    await fetchBoxes()
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '操作失败')
  } finally {
    submitting.value = false
  }
}

const toggleEnabled = async (row) => {
  const newVal = row.enabled ? 0 : 1
  try {
    await api.put(`/voicemail/boxes/${row.id}`, { enabled: newVal })
    ElMessage.success(newVal ? '已启用' : '已禁用')
    await fetchBoxes()
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '操作失败')
  }
}

onMounted(fetchBoxes)
onBeforeUnmount(clearAudioPreview)
</script>

<style scoped>
.mailbox-hint {
  font-size: 12px;
  color: var(--sip-text-3);
  margin-top: 2px;
}
.text-secondary { color: var(--sip-text-2); font-size: 13px; }
.audio-player { width: 100%; }
</style>
