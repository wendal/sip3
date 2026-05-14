import { createRouter, createWebHistory } from 'vue-router'
import Dashboard from '../views/Dashboard.vue'
import Accounts from '../views/Accounts.vue'
import Status from '../views/Status.vue'
import Login from '../views/Login.vue'
import AdminUsers from '../views/AdminUsers.vue'
import Acl from '../views/Acl.vue'
import Phone from '../views/Phone.vue'
import Security from '../views/Security.vue'

const routes = [
  { path: '/login', component: Login, meta: { public: true } },
  { path: '/phone', component: Phone, meta: { public: true } },
  { path: '/', redirect: '/dashboard' },
  { path: '/dashboard', component: Dashboard },
  { path: '/accounts', component: Accounts },
  { path: '/status', component: Status },
  { path: '/admin-users', component: AdminUsers },
  { path: '/acl', component: Acl },
  { path: '/security', component: Security },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
})

router.beforeEach((to) => {
  const token = localStorage.getItem('sip3_admin_token')
  if (!to.meta.public && !token) {
    return '/login'
  }
  if (to.path === '/login' && token) {
    return '/dashboard'
  }
})

export default router
