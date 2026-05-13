<template>
  <div>
    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
      <h2 style="margin: 0;">SIP Accounts</h2>
      <el-button type="primary" @click="openCreate">
        <el-icon><Plus /></el-icon> Add Account
      </el-button>
    </div>

    <el-card>
      <el-table :data="store.accounts" v-loading="store.loading" stripe>
        <el-table-column prop="id" label="ID" width="80" />
        <el-table-column prop="username" label="Username" />
        <el-table-column prop="display_name" label="Display Name" />
        <el-table-column prop="domain" label="Domain" />
        <el-table-column prop="enabled" label="Status" width="100">
          <template #default="{ row }">
            <el-tag :type="row.enabled ? 'success' : 'danger'">
              {{ row.enabled ? 'Enabled' : 'Disabled' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="created_at" label="Created" width="160">
          <template #default="{ row }">{{ formatDate(row.created_at) }}</template>
        </el-table-column>
        <el-table-column prop="last_call_at" label="Last Call" width="160">
          <template #default="{ row }">
            <span v-if="row.last_call_at">{{ formatDate(row.last_call_at) }}</span>
            <span v-else style="color: #bbb;">—</span>
          </template>
        </el-table-column>
        <el-table-column prop="call_count" label="Calls" width="70" align="center" />
        <el-table-column label="Actions" width="160">
          <template #default="{ row }">
            <el-button size="small" @click="openEdit(row)">Edit</el-button>
            <el-button size="small" type="danger" @click="handleDelete(row)">Delete</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- Create/Edit Dialog -->
    <el-dialog v-model="dialogVisible" :title="editingId ? 'Edit Account' : 'Add Account'" width="500px">
      <el-form :model="form" label-width="120px">
        <el-form-item label="Username" v-if="!editingId">
          <el-input v-model="form.username" placeholder="alice" />
        </el-form-item>
        <el-form-item label="Password">
          <el-input v-model="form.password" type="password" placeholder="Leave blank to keep current" />
        </el-form-item>
        <el-form-item label="Display Name">
          <el-input v-model="form.display_name" placeholder="Alice" />
        </el-form-item>
        <el-form-item label="Domain">
          <el-input v-model="form.domain" placeholder="sip.air32.cn" />
        </el-form-item>
        <el-form-item label="Status" v-if="editingId">
          <el-switch v-model="form.enabled" :active-value="1" :inactive-value="0" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false">Cancel</el-button>
        <el-button type="primary" @click="handleSubmit" :loading="submitting">
          {{ editingId ? 'Update' : 'Create' }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useSipStore } from '../store'

const store = useSipStore()
const dialogVisible = ref(false)
const editingId = ref(null)
const submitting = ref(false)
const form = ref({ username: '', password: '', display_name: '', domain: 'sip.air32.cn', enabled: 1 })

const formatDate = (d) => d ? new Date(d).toLocaleString() : '-'

const openCreate = () => {
  editingId.value = null
  form.value = { username: '', password: '', display_name: '', domain: 'sip.air32.cn', enabled: 1 }
  dialogVisible.value = true
}

const openEdit = (row) => {
  editingId.value = row.id
  form.value = { password: '', display_name: row.display_name || '', domain: row.domain, enabled: row.enabled }
  dialogVisible.value = true
}

const handleSubmit = async () => {
  try {
    submitting.value = true
    if (editingId.value) {
      const payload = { display_name: form.value.display_name, domain: form.value.domain, enabled: form.value.enabled }
      if (form.value.password) payload.password = form.value.password
      await store.updateAccount(editingId.value, payload)
      ElMessage.success('Account updated')
    } else {
      if (!form.value.username || !form.value.password) {
        ElMessage.error('Username and password are required')
        return
      }
      await store.createAccount({ username: form.value.username, password: form.value.password, display_name: form.value.display_name, domain: form.value.domain })
      ElMessage.success('Account created')
    }
    dialogVisible.value = false
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || 'Operation failed')
  } finally {
    submitting.value = false
  }
}

const handleDelete = async (row) => {
  await ElMessageBox.confirm(`Delete account "${row.username}"?`, 'Confirm', { type: 'warning' })
  try {
    await store.deleteAccount(row.id)
    ElMessage.success('Account deleted')
  } catch (e) {
    ElMessage.error(e.message)
  }
}

onMounted(() => store.fetchAccounts())
</script>
