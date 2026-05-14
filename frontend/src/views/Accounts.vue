<template>
  <div>
    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
      <h2 style="margin: 0;">SIP 账号管理</h2>
      <el-button type="primary" @click="openCreate">
        <el-icon><Plus /></el-icon> 添加账号
      </el-button>
    </div>

    <el-card>
      <div style="margin-bottom: 12px;">
        <el-input
          v-model="searchText"
          placeholder="按用户名或域搜索..."
          style="width: 260px;"
          clearable
        >
          <template #prefix><el-icon><Search /></el-icon></template>
        </el-input>
      </div>

      <el-table :data="filteredAccounts" v-loading="store.loading" stripe>
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column prop="username" label="用户名" />
        <el-table-column prop="display_name" label="显示名称" />
        <el-table-column prop="domain" label="域" />
        <el-table-column prop="enabled" label="状态" width="100">
          <template #default="{ row }">
            <el-switch
              :model-value="!!row.enabled"
              @change="toggleEnabled(row)"
              active-text="启用"
              inactive-text="禁用"
              inline-prompt
            />
          </template>
        </el-table-column>
        <el-table-column prop="created_at" label="创建时间" width="160">
          <template #default="{ row }">{{ formatDate(row.created_at) }}</template>
        </el-table-column>
        <el-table-column prop="last_call_at" label="最后通话" width="160">
          <template #default="{ row }">
            <span v-if="row.last_call_at">{{ formatDate(row.last_call_at) }}</span>
            <span v-else style="color: #bbb;">—</span>
          </template>
        </el-table-column>
        <el-table-column prop="call_count" label="通话次数" width="80" align="center" />
        <el-table-column label="操作" width="160">
          <template #default="{ row }">
            <el-button size="small" @click="openEdit(row)">编辑</el-button>
            <el-button size="small" type="danger" @click="handleDelete(row)">删除</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- Create/Edit Dialog -->
    <el-dialog v-model="dialogVisible" :title="editingId ? '编辑账号' : '添加账号'" width="500px">
      <el-form :model="form" label-width="90px">
        <el-form-item label="用户名" v-if="!editingId">
          <el-input v-model="form.username" placeholder="1001（3-6 位数字分机号）" />
        </el-form-item>
        <el-form-item label="密码">
          <el-input v-model="form.password" type="password" show-password
            :placeholder="editingId ? '留空则不修改' : '请输入密码'" />
        </el-form-item>
        <el-form-item label="显示名称">
          <el-input v-model="form.display_name" placeholder="Alice" />
        </el-form-item>
        <el-form-item label="域">
          <el-input v-model="form.domain" placeholder="sip.air32.cn" />
        </el-form-item>
        <el-form-item label="状态" v-if="editingId">
          <el-switch v-model="form.enabled" :active-value="1" :inactive-value="0"
            active-text="启用" inactive-text="禁用" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false">取消</el-button>
        <el-button type="primary" @click="handleSubmit" :loading="submitting">
          {{ editingId ? '更新' : '创建' }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useSipStore } from '../store'
import { isValidSipUsername, SIP_USERNAME_RULE_MESSAGE } from '../utils/sipUsername.mjs'

const store = useSipStore()
const searchText = ref('')
const dialogVisible = ref(false)
const editingId = ref(null)
const submitting = ref(false)
const form = ref({ username: '', password: '', display_name: '', domain: 'sip.air32.cn', enabled: 1 })

const formatDate = (d) => d ? new Date(d).toLocaleString() : '-'

const filteredAccounts = computed(() => {
  if (!searchText.value) return store.accounts
  const q = searchText.value.toLowerCase()
  return store.accounts.filter(a =>
    a.username.toLowerCase().includes(q) || a.domain.toLowerCase().includes(q)
  )
})

const toggleEnabled = async (row) => {
  const newVal = row.enabled ? 0 : 1
  try {
    await store.updateAccount(row.id, { domain: row.domain, enabled: newVal })
    ElMessage.success(newVal ? '账号已启用' : '账号已禁用')
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '操作失败')
  }
}

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
      ElMessage.success('账号已更新')
    } else {
      if (!form.value.username || !form.value.password) {
        ElMessage.error('用户名和密码不能为空')
        return
      }
      const username = form.value.username.trim()
      if (!isValidSipUsername(username)) {
        ElMessage.error(SIP_USERNAME_RULE_MESSAGE)
        return
      }
      await store.createAccount({
        username,
        password: form.value.password,
        display_name: form.value.display_name,
        domain: form.value.domain,
      })
      ElMessage.success('账号已创建')
    }
    dialogVisible.value = false
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '操作失败')
  } finally {
    submitting.value = false
  }
}

const handleDelete = async (row) => {
  await ElMessageBox.confirm(`确认删除账号 "${row.username}"？`, '确认删除', { type: 'warning' })
  try {
    await store.deleteAccount(row.id)
    ElMessage.success('账号已删除')
  } catch (e) {
    ElMessage.error(e.message)
  }
}

onMounted(() => store.fetchAccounts())
</script>
