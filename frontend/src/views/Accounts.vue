<template>
  <div class="sip-page">
    <PageHeader title="SIP 账号管理" subtitle="3-6 位数字分机号；密码同时维护 bcrypt 与 SIP HA1 摘要">
      <template #actions>
        <el-input
          v-model="searchText"
          placeholder="按用户名或域搜索…"
          style="width: 240px;"
          clearable
          :prefix-icon="Search"
        />
        <el-button type="primary" :icon="Plus" @click="openCreate">添加账号</el-button>
      </template>
    </PageHeader>

    <el-card>
      <el-table :data="filteredAccounts" v-loading="store.loading">
        <el-table-column prop="id" label="ID" width="70" />
        <el-table-column prop="username" label="用户名" min-width="120">
          <template #default="{ row }">
            <div class="username-cell">
              <div class="username-cell__avatar">{{ (row.display_name || row.username || '?').slice(0, 1).toUpperCase() }}</div>
              <div>
                <div class="username-cell__primary">{{ row.username }}</div>
                <div v-if="row.display_name" class="username-cell__secondary">{{ row.display_name }}</div>
              </div>
            </div>
          </template>
        </el-table-column>
        <el-table-column prop="domain" label="域" min-width="140" show-overflow-tooltip />
        <el-table-column prop="enabled" label="状态" width="100">
          <template #default="{ row }">
            <el-switch
              :model-value="!!row.enabled"
              @change="toggleEnabled(row)"
              inline-prompt active-text="ON" inactive-text="OFF"
            />
          </template>
        </el-table-column>
        <el-table-column prop="created_at" label="创建时间" width="160">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.created_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="last_call_at" label="最后通话" width="160">
          <template #default="{ row }">
            <span v-if="row.last_call_at" class="text-secondary">{{ formatDate(row.last_call_at) }}</span>
            <span v-else class="text-muted">—</span>
          </template>
        </el-table-column>
        <el-table-column prop="call_count" label="通话次数" width="90" align="right" />
        <el-table-column label="操作" width="120" fixed="right">
          <template #default="{ row }">
            <el-button text type="primary" @click="openEdit(row)">编辑</el-button>
            <el-button text type="danger" @click="handleDelete(row)">删除</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <EmptyState title="暂无账号" subtitle='点击右上角"添加账号"创建第一个 SIP 分机' />
        </template>
      </el-table>
    </el-card>

    <!-- Create/Edit Drawer -->
    <el-drawer v-model="dialogVisible" :title="editingId ? '编辑账号' : '添加账号'" size="420px" direction="rtl">
      <el-form :model="form" label-width="90px" label-position="top">
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
          <el-input v-model="form.domain" placeholder="sip.example.com" />
        </el-form-item>
        <el-form-item label="状态" v-if="editingId">
          <el-switch v-model="form.enabled" :active-value="1" :inactive-value="0"
            inline-prompt active-text="启用" inactive-text="禁用" />
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
import { ref, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Search } from '@element-plus/icons-vue'
import { useSipStore } from '../store'
import { isValidSipUsername, SIP_USERNAME_RULE_MESSAGE } from '../utils/sipUsername.mjs'
import PageHeader from '../components/PageHeader.vue'
import EmptyState from '../components/EmptyState.vue'

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

<style scoped>
.username-cell {
  display: flex;
  align-items: center;
  gap: 10px;
}
.username-cell__avatar {
  width: 32px; height: 32px;
  border-radius: 50%;
  background: linear-gradient(135deg, var(--sip-primary), #5e5ce6);
  color: #fff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 600;
  flex-shrink: 0;
}
.username-cell__primary { font-weight: 500; color: var(--sip-text); }
.username-cell__secondary { font-size: 12px; color: var(--sip-text-2); }
.text-secondary { color: var(--sip-text-2); font-size: 13px; }
.text-muted { color: var(--sip-text-3); }
</style>
