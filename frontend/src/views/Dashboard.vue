<template>
  <div>
    <h2 style="margin-bottom: 20px;">Dashboard</h2>

    <el-row :gutter="16" style="margin-bottom: 20px;">
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #409eff;">{{ store.totalAccounts }}</div>
            <div style="color: #666; margin-top: 8px;">Total Accounts</div>
          </div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #67c23a;">{{ store.enabledAccounts }}</div>
            <div style="color: #666; margin-top: 8px;">Enabled Accounts</div>
          </div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #e6a23c;">{{ store.activeRegistrations }}</div>
            <div style="color: #666; margin-top: 8px;">Active Registrations</div>
          </div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #f56c6c;">{{ store.activeCalls }}</div>
            <div style="color: #666; margin-top: 8px;">Active Calls</div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <el-row :gutter="16">
      <el-col :span="12">
        <el-card>
          <template #header>
            <span>Active Registrations</span>
            <el-button style="float: right;" text @click="store.fetchRegistrations()">
              <el-icon><Refresh /></el-icon>
            </el-button>
          </template>
          <el-table :data="store.registrations" size="small">
            <el-table-column prop="username" label="User" />
            <el-table-column prop="contact_uri" label="Contact" show-overflow-tooltip />
            <el-table-column prop="source_ip" label="IP" />
          </el-table>
        </el-card>
      </el-col>

      <el-col :span="12">
        <el-card>
          <template #header>
            <span>Recent Calls</span>
            <el-button style="float: right;" text @click="store.fetchCalls()">
              <el-icon><Refresh /></el-icon>
            </el-button>
          </template>
          <el-table :data="store.recentCalls" size="small">
            <el-table-column prop="caller" label="Caller" show-overflow-tooltip />
            <el-table-column prop="callee" label="Callee" show-overflow-tooltip />
            <el-table-column prop="status" label="Status">
              <template #default="{ row }">
                <el-tag :type="statusColor(row.status)" size="small">{{ row.status }}</el-tag>
              </template>
            </el-table-column>
          </el-table>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup>
import { onMounted } from 'vue'
import { useSipStore } from '../store'

const store = useSipStore()

const statusColor = (status) => {
  const map = { answered: 'success', ended: 'info', cancelled: 'warning', trying: '', failed: 'danger' }
  return map[status] || ''
}

onMounted(() => store.fetchAll())
</script>
