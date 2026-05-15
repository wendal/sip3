<template>
  <div class="sip-page">
    <PageHeader title="管理员账号" subtitle="管理后台登录账号，密码使用 bcrypt 哈希存储">
      <template #actions>
        <el-button type="primary" :icon="Plus" @click="openCreate">添加管理员</el-button>
      </template>
    </PageHeader>

    <el-card>
      <el-table :data="users" v-loading="loading">
        <el-table-column prop="id" label="ID" width="80" />
        <el-table-column prop="username" label="用户名" min-width="160">
          <template #default="{ row }">
            <div class="username-cell">
              <div class="username-cell__avatar">{{ (row.username || '?').slice(0, 1).toUpperCase() }}</div>
              <div class="username-cell__primary">{{ row.username }}</div>
            </div>
          </template>
        </el-table-column>
        <el-table-column prop="created_at" label="创建时间" width="200">
          <template #default="{ row }">
            <span class="text-secondary">{{ formatDate(row.created_at) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="200" fixed="right">
          <template #default="{ row }">
            <el-button text type="primary" @click="openChangePassword(row)">修改密码</el-button>
            <el-button text type="danger" @click="handleDelete(row)">删除</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <EmptyState title="暂无管理员" />
        </template>
      </el-table>
    </el-card>

    <el-drawer v-model="createDialogVisible" title="添加管理员" size="400px" direction="rtl">
      <el-form :model="createForm" label-position="top">
        <el-form-item label="用户名">
          <el-input v-model="createForm.username" placeholder="输入用户名" />
        </el-form-item>
        <el-form-item label="密码">
          <el-input v-model="createForm.password" type="password" show-password placeholder="至少 6 位" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="createDialogVisible = false">取消</el-button>
        <el-button type="primary" @click="handleCreate" :loading="submitting">创建</el-button>
      </template>
    </el-drawer>

    <el-drawer v-model="pwdDialogVisible" title="修改密码" size="400px" direction="rtl">
      <el-form :model="pwdForm" label-position="top">
        <el-form-item label="账号">
          <span class="text-secondary">{{ pwdForm.username }}</span>
        </el-form-item>
        <el-form-item label="新密码">
          <el-input v-model="pwdForm.password" type="password" show-password placeholder="至少 6 位" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="pwdDialogVisible = false">取消</el-button>
        <el-button type="primary" @click="handleChangePassword" :loading="submitting">确认修改</el-button>
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
import EmptyState from '../components/EmptyState.vue'

const users = ref([])
const loading = ref(false)
const submitting = ref(false)
const createDialogVisible = ref(false)
const pwdDialogVisible = ref(false)
const createForm = ref({ username: '', password: '' })
const pwdForm = ref({ id: null, username: '', password: '' })

const formatDate = (d) => d ? new Date(d).toLocaleString() : '-'

const fetchUsers = async () => {
  try {
    loading.value = true
    const res = await api.get('/admin/users')
    users.value = res.data.data || []
  } catch (e) {
    ElMessage.error('加载失败: ' + (e.response?.data || e.message))
  } finally {
    loading.value = false
  }
}

const openCreate = () => {
  createForm.value = { username: '', password: '' }
  createDialogVisible.value = true
}

const handleCreate = async () => {
  if (!createForm.value.username || !createForm.value.password) {
    ElMessage.warning('用户名和密码不能为空')
    return
  }
  try {
    submitting.value = true
    await api.post('/admin/users', createForm.value)
    ElMessage.success('管理员创建成功')
    createDialogVisible.value = false
    await fetchUsers()
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '创建失败')
  } finally {
    submitting.value = false
  }
}

const openChangePassword = (row) => {
  pwdForm.value = { id: row.id, username: row.username, password: '' }
  pwdDialogVisible.value = true
}

const handleChangePassword = async () => {
  if (!pwdForm.value.password) {
    ElMessage.warning('请输入新密码')
    return
  }
  try {
    submitting.value = true
    await api.put(`/admin/users/${pwdForm.value.id}`, { password: pwdForm.value.password })
    ElMessage.success('密码修改成功')
    pwdDialogVisible.value = false
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '修改失败')
  } finally {
    submitting.value = false
  }
}

const handleDelete = async (row) => {
  await ElMessageBox.confirm(`确定删除管理员 "${row.username}" 吗？此操作不可撤销。`, '确认删除', { type: 'warning' })
  try {
    await api.delete(`/admin/users/${row.id}`)
    ElMessage.success('已删除')
    await fetchUsers()
  } catch (e) {
    ElMessage.error(e.response?.data || e.message || '删除失败')
  }
}

onMounted(fetchUsers)
</script>

<style scoped>
.username-cell { display: flex; align-items: center; gap: 10px; }
.username-cell__avatar {
  width: 32px; height: 32px;
  border-radius: 50%;
  background: linear-gradient(135deg, var(--sip-warning), #ff6b35);
  color: #fff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 600;
}
.username-cell__primary { font-weight: 500; color: var(--sip-text); }
.text-secondary { color: var(--sip-text-2); font-size: 13px; }
</style>
