<template>
  <div class="sip-page">
    <PageHeader title="IP ACL 规则" subtitle="按优先级匹配，首条命中规则生效。无规则匹配时使用默认策略">
      <template #actions>
        <el-button type="primary" :icon="Plus" @click="openCreate">添加规则</el-button>
      </template>
    </PageHeader>

    <el-alert
      type="info"
      :closable="false"
      style="margin-bottom: 16px; border-radius: var(--sip-radius);"
      show-icon
    >
      <template #title>
        规则按优先级（数值越小越优先）匹配，首条命中生效。无规则匹配时使用服务器默认策略（默认：allow）。
      </template>
    </el-alert>

    <el-card>
      <el-table :data="acls" v-loading="loading">
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column prop="action" label="动作" width="100">
          <template #default="{ row }">
            <StatusTag :status="row.action" />
          </template>
        </el-table-column>
        <el-table-column prop="cidr" label="CIDR" width="200">
          <template #default="{ row }"><code class="cidr-cell">{{ row.cidr }}</code></template>
        </el-table-column>
        <el-table-column prop="priority" label="优先级" width="90" align="center" />
        <el-table-column prop="enabled" label="状态" width="100">
          <template #default="{ row }">
            <StatusTag :status="row.enabled ? 'enabled' : 'disabled'" />
          </template>
        </el-table-column>
        <el-table-column prop="description" label="备注" show-overflow-tooltip />
        <el-table-column prop="created_at" label="创建时间" width="160">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.created_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="120" fixed="right">
          <template #default="{ row }">
            <el-button text type="primary" @click="openEdit(row)">编辑</el-button>
            <el-button text type="danger" @click="handleDelete(row)">删除</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <EmptyState title="暂无规则" subtitle="尚未配置任何 IP ACL 规则" />
        </template>
      </el-table>
    </el-card>

    <el-drawer v-model="dialogVisible" :title="editingId ? '编辑规则' : '添加规则'" size="400px" direction="rtl">
      <el-form :model="form" label-position="top">
        <el-form-item label="动作">
          <el-radio-group v-model="form.action">
            <el-radio-button label="allow">Allow（放行）</el-radio-button>
            <el-radio-button label="deny">Deny（拒绝）</el-radio-button>
          </el-radio-group>
        </el-form-item>
        <el-form-item label="CIDR">
          <el-input v-model="form.cidr" placeholder="192.168.1.0/24 或 10.0.0.1/32" />
        </el-form-item>
        <el-form-item label="优先级">
          <el-input-number v-model="form.priority" :min="0" :max="9999" />
          <div class="form-hint">数值越小越优先</div>
        </el-form-item>
        <el-form-item label="状态">
          <el-switch v-model="form.enabled" :active-value="1" :inactive-value="0"
            inline-prompt active-text="启用" inactive-text="禁用" />
        </el-form-item>
        <el-form-item label="备注">
          <el-input v-model="form.description" type="textarea" :rows="2" placeholder="可选备注" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false">取消</el-button>
        <el-button type="primary" @click="handleSubmit" :loading="submitting">
          {{ editingId ? '更新' : '创建' }}
        </el-button>
      </template>
    </el-drawer>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus } from '@element-plus/icons-vue'
import api from '../utils/api'
import PageHeader from '../components/PageHeader.vue'
import StatusTag from '../components/StatusTag.vue'
import EmptyState from '../components/EmptyState.vue'

const acls = ref([])
const loading = ref(false)
const dialogVisible = ref(false)
const editingId = ref(null)
const submitting = ref(false)
const form = ref({ action: 'deny', cidr: '', priority: 100, enabled: 1, description: '' })

const formatDate = (d) => d ? new Date(d).toLocaleString() : '-'

const fetchAcls = async () => {
  loading.value = true
  try {
    const res = await api.get('/acl')
    acls.value = res.data.data || []
  } finally {
    loading.value = false
  }
}

const openCreate = () => {
  editingId.value = null
  form.value = { action: 'deny', cidr: '', priority: 100, enabled: 1, description: '' }
  dialogVisible.value = true
}

const openEdit = (row) => {
  editingId.value = row.id
  form.value = {
    action: row.action,
    cidr: row.cidr,
    priority: row.priority,
    enabled: row.enabled,
    description: row.description || '',
  }
  dialogVisible.value = true
}

const handleSubmit = async () => {
  if (!form.value.cidr.trim()) {
    ElMessage.error('请输入 CIDR')
    return
  }
  try {
    submitting.value = true
    const payload = {
      action: form.value.action,
      cidr: form.value.cidr.trim(),
      priority: form.value.priority,
      enabled: form.value.enabled,
      description: form.value.description || null,
    }
    if (editingId.value) {
      await api.put(`/acl/${editingId.value}`, payload)
      ElMessage.success('规则已更新')
    } else {
      await api.post('/acl', payload)
      ElMessage.success('规则已创建')
    }
    dialogVisible.value = false
    await fetchAcls()
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '操作失败')
  } finally {
    submitting.value = false
  }
}

const handleDelete = async (row) => {
  await ElMessageBox.confirm(`确认删除规则 "${row.action} ${row.cidr}"？`, '确认删除', { type: 'warning' })
  try {
    await api.delete(`/acl/${row.id}`)
    ElMessage.success('规则已删除')
    await fetchAcls()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

onMounted(fetchAcls)
</script>

<style scoped>
.cidr-cell {
  font-family: 'SF Mono', 'Cascadia Code', Consolas, monospace;
  font-size: 12px;
  background: var(--sip-surface-2);
  padding: 2px 8px;
  border-radius: 6px;
  color: var(--sip-text);
}
.text-secondary { color: var(--sip-text-2); font-size: 13px; }
.form-hint { font-size: 12px; color: var(--sip-text-3); margin-top: 4px; }
</style>
