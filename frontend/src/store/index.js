import { defineStore } from 'pinia'
import axios from 'axios'

const api = axios.create({ baseURL: '/api' })

export const useSipStore = defineStore('sip', {
  state: () => ({
    accounts: [],
    registrations: [],
    calls: [],
    loading: false,
    error: null,
  }),

  getters: {
    activeRegistrations: (state) => state.registrations.length,
    totalAccounts: (state) => state.accounts.length,
    enabledAccounts: (state) => state.accounts.filter(a => a.enabled === 1).length,
    recentCalls: (state) => state.calls.slice(0, 10),
    activeCalls: (state) => state.calls.filter(c => c.status === 'answered').length,
  },

  actions: {
    async fetchAccounts() {
      try {
        this.loading = true
        const res = await api.get('/accounts')
        this.accounts = res.data.data || []
      } catch (e) {
        this.error = e.message
      } finally {
        this.loading = false
      }
    },

    async createAccount(data) {
      const res = await api.post('/accounts', data)
      await this.fetchAccounts()
      return res.data
    },

    async updateAccount(id, data) {
      const res = await api.put(`/accounts/${id}`, data)
      await this.fetchAccounts()
      return res.data
    },

    async deleteAccount(id) {
      await api.delete(`/accounts/${id}`)
      await this.fetchAccounts()
    },

    async fetchRegistrations() {
      try {
        const res = await api.get('/registrations')
        this.registrations = res.data.data || []
      } catch (e) {
        this.error = e.message
      }
    },

    async fetchCalls() {
      try {
        const res = await api.get('/calls')
        this.calls = res.data.data || []
      } catch (e) {
        this.error = e.message
      }
    },

    async fetchAll() {
      await Promise.all([
        this.fetchAccounts(),
        this.fetchRegistrations(),
        this.fetchCalls(),
      ])
    },
  },
})
