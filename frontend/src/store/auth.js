import { defineStore } from 'pinia'
import api from '../utils/api'

const TOKEN_KEY = 'sip3_admin_token'
const USERNAME_KEY = 'sip3_admin_username'

export const useAuthStore = defineStore('auth', {
  state: () => ({
    token: localStorage.getItem(TOKEN_KEY) || null,
    username: localStorage.getItem(USERNAME_KEY) || null,
  }),

  getters: {
    isAuthenticated: (state) => !!state.token,
  },

  actions: {
    async login(username, password) {
      const res = await api.post('/auth/login', { username, password })
      this.token = res.data.token
      this.username = res.data.username
      localStorage.setItem(TOKEN_KEY, this.token)
      localStorage.setItem(USERNAME_KEY, this.username)
    },

    logout() {
      this.token = null
      this.username = null
      localStorage.removeItem(TOKEN_KEY)
      localStorage.removeItem(USERNAME_KEY)
    },
  },
})
