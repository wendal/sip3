<template>
  <div class="sip-page">
    <PageHeader title="控制台" subtitle="服务器运行概览与近期通话趋势">
      <template #actions>
        <el-button :icon="Refresh" @click="refresh" :loading="store.loading">刷新</el-button>
      </template>
    </PageHeader>

    <!-- KPI grid -->
    <div class="sip-stat-grid kpi-grid">
      <StatCard :icon="User"        tone="primary" label="账号总数"   :value="store.totalAccounts" />
      <StatCard :icon="CircleCheck" tone="success" label="启用账号"   :value="store.enabledAccounts" />
      <StatCard :icon="Connection"  tone="warning" label="当前注册"   :value="store.activeRegistrations" />
      <StatCard :icon="PhoneFilled" tone="danger"  label="活跃通话"   :value="store.activeCalls" />
    </div>

    <div v-if="store.stats" class="sip-stat-grid kpi-grid" style="margin-top: 16px;">
      <StatCard :icon="Bell"      tone="primary" label="今日呼叫"
                :value="store.stats.today.calls"
                :hint="`接通 ${store.stats.today.answered} · ${answerRate(store.stats.today)}%`" />
      <StatCard :icon="Calendar"  tone="success" label="近7天呼叫"
                :value="store.stats.week.calls"
                :hint="`接通 ${store.stats.week.answered} · ${answerRate(store.stats.week)}%`" />
      <StatCard :icon="Timer"     tone="warning" label="今日通话时长"
                :value="fmtDuration(store.stats.today.duration_secs)" />
      <StatCard :icon="DataAnalysis" tone="info" label="近30天平均时长"
                :value="fmtDuration(store.stats.avg_duration_secs)" />
    </div>

    <!-- 24h chart + answer-rate gauge -->
    <el-row v-if="store.stats" :gutter="16" style="margin-top: 16px;">
      <el-col :xs="24" :lg="16">
        <el-card>
          <template #header>
            <div class="card-head">
              <span>近 24 小时呼叫分布</span>
              <span class="card-head__hint">峰值 {{ maxHourly }} 通</span>
            </div>
          </template>
          <div class="bar-chart">
            <div
              v-for="(count, hour) in store.stats.hourly_calls"
              :key="hour"
              class="bar-col"
              :title="`${hour}:00 - ${count} 通`"
            >
              <div class="bar-col__count">{{ count || '' }}</div>
              <div class="bar-col__track">
                <div
                  class="bar-col__fill"
                  :class="{ 'bar-col__fill--zero': !count }"
                  :style="{ height: count > 0 ? `${Math.max(6, Math.round(count / maxHourly * 100))}%` : '4px' }"
                />
              </div>
              <div class="bar-col__label">{{ hour % 4 === 0 ? `${hour}:00` : '' }}</div>
            </div>
          </div>
        </el-card>
      </el-col>
      <el-col :xs="24" :lg="8">
        <el-card>
          <template #header>
            <span>近 7 天接通率</span>
          </template>
          <div class="gauge-wrap">
            <div class="gauge" :style="{ '--p': weeklyAnswerRate }">
              <div class="gauge__value num">{{ weeklyAnswerRate }}<span class="gauge__pct">%</span></div>
              <div class="gauge__sub">{{ store.stats.week.answered }} / {{ store.stats.week.calls }}</div>
            </div>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <!-- Registrations + recent calls -->
    <el-row :gutter="16" style="margin-top: 16px;">
      <el-col :xs="24" :lg="12">
        <el-card>
          <template #header>
            <div class="card-head">
              <span>当前注册 ({{ store.registrations.length }})</span>
              <el-button text :icon="Refresh" @click="store.fetchRegistrations()" />
            </div>
          </template>
          <el-table :data="store.registrations" size="small" :max-height="280">
            <el-table-column prop="username" label="用户" width="100" />
            <el-table-column prop="contact_uri" label="Contact" show-overflow-tooltip />
            <el-table-column prop="source_ip" label="IP" width="120" />
            <template #empty>
              <EmptyState title="暂无注册" subtitle="尚无 SIP 客户端注册到本服务器" />
            </template>
          </el-table>
        </el-card>
      </el-col>

      <el-col :xs="24" :lg="12">
        <el-card>
          <template #header>
            <div class="card-head">
              <span>最近通话</span>
              <el-button text :icon="Refresh" @click="store.fetchCalls()" />
            </div>
          </template>
          <el-table :data="store.recentCalls" size="small" :max-height="280">
            <el-table-column prop="caller" label="主叫" show-overflow-tooltip />
            <el-table-column prop="callee" label="被叫" show-overflow-tooltip />
            <el-table-column label="状态" width="90">
              <template #default="{ row }">
                <StatusTag :status="row.status" />
              </template>
            </el-table-column>
            <template #empty>
              <EmptyState title="暂无通话" subtitle="发起或接听一次呼叫，将显示在此" />
            </template>
          </el-table>
        </el-card>
      </el-col>
    </el-row>

    <!-- Top callers -->
    <el-row v-if="store.stats && (store.stats.top_callers.length || store.stats.top_callees.length)"
            :gutter="16" style="margin-top: 16px;">
      <el-col :xs="24" :lg="12">
        <el-card>
          <template #header><span>近 7 天 Top 主叫</span></template>
          <el-table :data="store.stats.top_callers" size="small">
            <el-table-column prop="user" label="用户" show-overflow-tooltip />
            <el-table-column prop="count" label="拨出次数" width="100" align="right" />
          </el-table>
        </el-card>
      </el-col>
      <el-col :xs="24" :lg="12">
        <el-card>
          <template #header><span>近 7 天 Top 被叫</span></template>
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
import {
  User, CircleCheck, Connection, PhoneFilled, Refresh,
  Bell, Calendar, Timer, DataAnalysis,
} from '@element-plus/icons-vue'
import { useSipStore } from '../store'
import PageHeader from '../components/PageHeader.vue'
import StatCard from '../components/StatCard.vue'
import StatusTag from '../components/StatusTag.vue'
import EmptyState from '../components/EmptyState.vue'

const store = useSipStore()

const maxHourly = computed(() => {
  if (!store.stats) return 1
  return Math.max(1, ...store.stats.hourly_calls)
})

const answerRate = (period) => {
  if (!period.calls) return 0
  return Math.round((period.answered / period.calls) * 100)
}

const weeklyAnswerRate = computed(() => {
  if (!store.stats) return 0
  return answerRate(store.stats.week)
})

const fmtDuration = (secs) => {
  if (!secs) return '0s'
  const s = Math.round(secs)
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m${s % 60}s`
  return `${Math.floor(s / 3600)}h${Math.floor((s % 3600) / 60)}m`
}

function refresh() { store.fetchAll() }

onMounted(() => store.fetchAll())
</script>

<style scoped>
.kpi-grid { grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); }

.card-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  width: 100%;
}
.card-head__hint {
  font-size: 12px;
  color: var(--sip-text-2);
  font-weight: 400;
}

/* 24h bar chart */
.bar-chart {
  display: flex;
  align-items: flex-end;
  gap: 4px;
  height: 160px;
  padding: 4px 4px 0;
}
.bar-col {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  min-width: 0;
}
.bar-col__count {
  font-size: 10px;
  color: var(--sip-text-2);
  height: 14px;
  line-height: 14px;
  font-variant-numeric: tabular-nums;
}
.bar-col__track {
  width: 100%;
  height: 110px;
  display: flex;
  align-items: flex-end;
  border-radius: 4px 4px 0 0;
  background: var(--sip-surface-2);
}
.bar-col__fill {
  width: 100%;
  background: linear-gradient(180deg, var(--sip-primary) 0%, #5e5ce6 100%);
  border-radius: 4px 4px 0 0;
  transition: height 0.4s cubic-bezier(0.4, 0, 0.2, 1);
}
.bar-col__fill--zero {
  background: var(--sip-border);
}
.bar-col__label {
  font-size: 9px;
  color: var(--sip-text-3);
  margin-top: 4px;
  height: 12px;
  line-height: 12px;
  font-variant-numeric: tabular-nums;
}

/* Gauge (CSS conic) */
.gauge-wrap {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 8px 0;
}
.gauge {
  --size: 160px;
  width: var(--size); height: var(--size);
  border-radius: 50%;
  position: relative;
  background:
    conic-gradient(var(--sip-success) calc(var(--p) * 1%), var(--sip-surface-2) 0);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-direction: column;
}
.gauge::before {
  content: '';
  position: absolute;
  inset: 12px;
  border-radius: 50%;
  background: var(--sip-surface);
}
.gauge__value {
  position: relative;
  font-size: 32px;
  font-weight: 700;
  color: var(--sip-text);
  letter-spacing: -1px;
}
.gauge__pct { font-size: 14px; font-weight: 500; color: var(--sip-text-2); margin-left: 2px; }
.gauge__sub {
  position: relative;
  font-size: 12px;
  color: var(--sip-text-2);
  margin-top: 2px;
}
</style>
