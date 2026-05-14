<template>
  <div>
    <h2 style="margin-bottom: 20px;">控制台</h2>

    <!-- 账号 & 呼叫概览 -->
    <el-row :gutter="16" style="margin-bottom: 20px;">
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #409eff;">{{ store.totalAccounts }}</div>
            <div style="color: #666; margin-top: 8px;">账号总数</div>
          </div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #67c23a;">{{ store.enabledAccounts }}</div>
            <div style="color: #666; margin-top: 8px;">启用账号</div>
          </div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #e6a23c;">{{ store.activeRegistrations }}</div>
            <div style="color: #666; margin-top: 8px;">当前注册</div>
          </div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #f56c6c;">{{ store.activeCalls }}</div>
            <div style="color: #666; margin-top: 8px;">活跃通话</div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 通话统计 -->
    <el-row :gutter="16" style="margin-bottom: 20px;" v-if="store.stats">
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #409eff;">{{ store.stats.today.calls }}</div>
            <div style="color: #666; margin-top: 4px;">今日呼叫</div>
            <div style="margin-top: 8px;">
              <el-progress
                :percentage="answerRate(store.stats.today)"
                :stroke-width="8"
                :format="() => `接通 ${store.stats.today.answered}`"
              />
            </div>
          </div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #67c23a;">{{ store.stats.week.calls }}</div>
            <div style="color: #666; margin-top: 4px;">近7天呼叫</div>
            <div style="margin-top: 8px;">
              <el-progress
                :percentage="answerRate(store.stats.week)"
                :stroke-width="8"
                :format="() => `接通 ${store.stats.week.answered}`"
              />
            </div>
          </div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #e6a23c;">{{ fmtDuration(store.stats.today.duration_secs) }}</div>
            <div style="color: #666; margin-top: 8px;">今日通话时长</div>
          </div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card shadow="hover">
          <div style="text-align: center;">
            <div style="font-size: 36px; font-weight: bold; color: #909399;">{{ fmtDuration(store.stats.avg_duration_secs) }}</div>
            <div style="color: #666; margin-top: 8px;">近30天平均时长</div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 近24小时逐小时呼叫量 -->
    <el-row :gutter="16" style="margin-bottom: 20px;" v-if="store.stats">
      <el-col :span="24">
        <el-card>
          <template #header>
            <span>近24小时呼叫分布</span>
            <el-button style="float: right;" text @click="store.fetchStats()">
              <el-icon><Refresh /></el-icon>
            </el-button>
          </template>
          <div style="display: flex; align-items: flex-end; gap: 4px; height: 80px; padding: 0 4px;">
            <div
              v-for="(count, hour) in store.stats.hourly_calls"
              :key="hour"
              style="flex: 1; display: flex; flex-direction: column; align-items: center;"
            >
              <div style="font-size: 10px; color: #666; margin-bottom: 2px;">{{ count || '' }}</div>
              <div
                :style="{
                  width: '100%',
                  background: count > 0 ? '#409eff' : '#eee',
                  borderRadius: '2px 2px 0 0',
                  height: count > 0 ? `${Math.max(8, Math.round(count / maxHourly * 52))}px` : '4px',
                  transition: 'height 0.3s',
                }"
              />
              <div style="font-size: 9px; color: #999; margin-top: 2px;">{{ hour % 4 === 0 ? `${hour}:00` : '' }}</div>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 注册列表 & 最近通话 -->
    <el-row :gutter="16" style="margin-bottom: 20px;">
      <el-col :span="12">
        <el-card>
          <template #header>
            <span>当前注册</span>
            <el-button style="float: right;" text @click="store.fetchRegistrations()">
              <el-icon><Refresh /></el-icon>
            </el-button>
          </template>
          <el-table :data="store.registrations" size="small">
            <el-table-column prop="username" label="用户" />
            <el-table-column prop="contact_uri" label="Contact" show-overflow-tooltip />
            <el-table-column prop="source_ip" label="IP" />
          </el-table>
        </el-card>
      </el-col>

      <el-col :span="12">
        <el-card>
          <template #header>
            <span>最近通话</span>
            <el-button style="float: right;" text @click="store.fetchCalls()">
              <el-icon><Refresh /></el-icon>
            </el-button>
          </template>
          <el-table :data="store.recentCalls" size="small">
            <el-table-column prop="caller" label="主叫" show-overflow-tooltip />
            <el-table-column prop="callee" label="被叫" show-overflow-tooltip />
            <el-table-column prop="status" label="状态" width="90">
              <template #default="{ row }">
                <el-tag :type="statusColor(row.status)" size="small">{{ statusLabel(row.status) }}</el-tag>
              </template>
            </el-table-column>
          </el-table>
        </el-card>
      </el-col>
    </el-row>

    <!-- Top 用户（近7天） -->
    <el-row :gutter="16" v-if="store.stats && (store.stats.top_callers.length || store.stats.top_callees.length)">
      <el-col :span="12">
        <el-card>
          <template #header><span>近7天 Top 主叫</span></template>
          <el-table :data="store.stats.top_callers" size="small">
            <el-table-column prop="user" label="用户" show-overflow-tooltip />
            <el-table-column prop="count" label="拨出次数" width="100" align="right" />
          </el-table>
        </el-card>
      </el-col>
      <el-col :span="12">
        <el-card>
          <template #header><span>近7天 Top 被叫</span></template>
          <el-table :data="store.stats.top_callees" size="small">
            <el-table-column prop="user" label="用户" show-overflow-tooltip />
            <el-table-column prop="count" label="接入次数" width="100" align="right" />
          </el-table>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup>
import { computed, onMounted } from 'vue'
import { useSipStore } from '../store'

const store = useSipStore()

const maxHourly = computed(() => {
  if (!store.stats) return 1
  return Math.max(1, ...store.stats.hourly_calls)
})

const answerRate = (period) => {
  if (!period.calls) return 0
  return Math.round((period.answered / period.calls) * 100)
}

const fmtDuration = (secs) => {
  if (!secs) return '0s'
  const s = Math.round(secs)
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m${s % 60}s`
  return `${Math.floor(s / 3600)}h${Math.floor((s % 3600) / 60)}m`
}

const statusColor = (status) => {
  const map = { answered: 'success', ended: 'info', cancelled: 'warning', trying: '', failed: 'danger' }
  return map[status] || ''
}

const statusLabel = (status) => {
  const map = { answered: '接通', ended: '结束', cancelled: '取消', trying: '呼叫中', failed: '失败' }
  return map[status] || status
}

onMounted(() => store.fetchAll())
</script>
