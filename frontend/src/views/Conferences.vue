<template>
  <div class="sip-page">
    <PageHeader
      title="会议室管理"
      subtitle="9 位数字会议号；MVP 仅支持 SIP UDP/TLS + G.711 (PCMU/PCMA)"
    >
      <template #actions>
        <el-button type="primary" :icon="Plus" @click="openCreate">添加会议室</el-button>
      </template>
    </PageHeader>

    <el-card>
      <el-table :data="rooms" v-loading="loading">
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column prop="extension" label="会议号" min-width="140">
          <template #default="{ row }">
            <code>{{ row.extension }}</code>
            <div class="dial-hint">sip:{{ row.extension }}@{{ row.domain }}</div>
          </template>
        </el-table-column>
        <el-table-column prop="name" label="名称" min-width="160" show-overflow-tooltip />
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
        <el-table-column prop="max_participants" label="人数上限" width="100" align="right" />
        <el-table-column prop="created_at" label="创建时间" width="170">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.created_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="220" fixed="right">
          <template #default="{ row }">
            <el-button text type="primary" @click="openEdit(row)">编辑</el-button>
            <el-button text @click="openParticipants(row)">参会者</el-button>
            <el-button text type="danger" @click="handleDelete(row)">删除</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <EmptyState title="暂无会议室" subtitle="点击右上角“添加会议室”创建第一间会议室" />
        </template>
      </el-table>
    </el-card>

    <!-- Create/Edit Drawer -->
    <el-drawer
      v-model="drawerVisible"
      :title="editingId ? '编辑会议室' : '添加会议室'"
      size="420px"
      direction="rtl"
    >
      <el-form :model="form" label-width="90px" label-position="top">
        <el-form-item label="会议号" v-if="!editingId">
          <el-input v-model="form.extension" placeholder="900000000（9 位数字）" maxlength="9" />
        </el-form-item>
        <el-form-item label="名称">
          <el-input v-model="form.name" placeholder="Default Conference" />
        </el-form-item>
        <el-form-item label="域">
          <el-input v-model="form.domain" placeholder="sip.example.com" />
        </el-form-item>
        <el-form-item label="人数上限">
          <el-input-number v-model="form.max_participants" :min="1" :max="200" />
        </el-form-item>
        <el-form-item label="状态" v-if="editingId">
          <el-switch
            v-model="form.enabled" :active-value="1" :inactive-value="0"
            inline-prompt active-text="启用" inactive-text="禁用"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="drawerVisible = false">取消</el-button>
        <el-button type="primary" @click="handleSubmit" :loading="submitting">
          {{ editingId ? '更新' : '创建' }}
        </el-button>
      </template>
    </el-drawer>

    <!-- Participants Drawer -->
    <el-drawer
      v-model="participantsVisible"
      :title="`参会者 — ${currentRoom?.extension || ''}`"
      size="640px"
      direction="rtl"
    >
      <el-table :data="participants" v-loading="participantsLoading">
        <el-table-column prop="account" label="账号" min-width="100" />
        <el-table-column prop="codec" label="编码" width="80" />
        <el-table-column label="来源" min-width="160">
          <template #default="{ row }">{{ row.source_ip }}:{{ row.source_port }}</template>
        </el-table-column>
        <el-table-column prop="relay_port" label="中继端口" width="100" align="right" />
        <el-table-column label="静音" width="80">
          <template #default="{ row }">
            <el-tag v-if="row.muted" type="warning" size="small">muted</el-tag>
            <el-tag v-else type="success" size="small">live</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="joined_at" label="加入时间" width="170">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.joined_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="left_at" label="离开时间" width="170">
          <template #default="{ row }">
            <span v-if="row.left_at" class="text-secondary">{{ formatDate(row.left_at) }}</span>
            <span v-else class="text-muted">在线</span>
          </template>
        </el-table-column>
        <template #empty>
          <EmptyState title="暂无参会者" subtitle="拨打会议号即可加入" />
        </template>
      </el-table>
    </el-drawer>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus } from '@element-plus/icons-vue'
import api from '../utils/api'
import PageHeader from '../components/PageHeader.vue'
import EmptyState from '../components/EmptyState.vue'

const rooms = ref([])
const loading = ref(false)
const drawerVisible = ref(false)
const editingId = ref(null)
const submitting = ref(false)
const form = ref(emptyForm())
const participantsVisible = ref(false)
const participantsLoading = ref(false)
const participants = ref([])
const currentRoom = ref(null)

function emptyForm() {
  return { extension: '', name: '', domain: 'sip.air32.cn', max_participants: 20, enabled: 1 }
}

const formatDate = (d) => d ? new Date(d).toLocaleString() : '-'

const fetchRooms = async () => {
  loading.value = true
  try {
    const res = await api.get('/conferences')
    rooms.value = res.data.data || []
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '加载失败')
  } finally {
    loading.value = false
  }
}

const openCreate = () => {
  editingId.value = null
  form.value = emptyForm()
  drawerVisible.value = true
}

const openEdit = (row) => {
  editingId.value = row.id
  form.value = {
    extension: row.extension,
    name: row.name,
    domain: row.domain,
    max_participants: row.max_participants,
    enabled: row.enabled,
  }
  drawerVisible.value = true
}

const handleSubmit = async () => {
  submitting.value = true
  try {
    if (editingId.value) {
      await api.put(`/conferences/${editingId.value}`, {
        name: form.value.name,
        domain: form.value.domain,
        max_participants: form.value.max_participants,
        enabled: form.value.enabled,
      })
      ElMessage.success('会议室已更新')
    } else {
      if (!/^\d{9}$/.test(form.value.extension)) {
        ElMessage.error('会议号必须为 9 位数字')
        submitting.value = false
        return
      }
      if (!form.value.name.trim()) {
        ElMessage.error('请填写会议室名称')
        submitting.value = false
        return
      }
      await api.post('/conferences', {
        extension: form.value.extension,
        name: form.value.name,
        domain: form.value.domain,
        max_participants: form.value.max_participants,
      })
      ElMessage.success('会议室已创建')
    }
    drawerVisible.value = false
    await fetchRooms()
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '操作失败')
  } finally {
    submitting.value = false
  }
}

const toggleEnabled = async (row) => {
  const newVal = row.enabled ? 0 : 1
  try {
    await api.put(`/conferences/${row.id}`, { enabled: newVal })
    ElMessage.success(newVal ? '已启用' : '已禁用')
    await fetchRooms()
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '操作失败')
  }
}

const handleDelete = async (row) => {
  await ElMessageBox.confirm(`确认删除会议室 "${row.extension}"？`, '确认删除', { type: 'warning' })
  try {
    await api.delete(`/conferences/${row.id}`)
    ElMessage.success('会议室已删除')
    await fetchRooms()
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '操作失败')
  }
}

const openParticipants = async (row) => {
  currentRoom.value = row
  participantsVisible.value = true
  participantsLoading.value = true
  try {
    const res = await api.get(`/conferences/${row.id}/participants`)
    participants.value = res.data.data || []
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '加载失败')
  } finally {
    participantsLoading.value = false
  }
}

onMounted(fetchRooms)
</script>

<style scoped>
.dial-hint {
  font-size: 12px;
  color: var(--sip-text-3);
  margin-top: 2px;
}
.text-secondary { color: var(--sip-text-2); font-size: 13px; }
.text-muted { color: var(--sip-text-3); }
</style>
