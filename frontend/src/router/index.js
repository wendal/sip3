import { createRouter, createWebHistory } from 'vue-router'
import Dashboard from '../views/Dashboard.vue'
import Accounts from '../views/Accounts.vue'
import Status from '../views/Status.vue'

const routes = [
  { path: '/', redirect: '/dashboard' },
  { path: '/dashboard', component: Dashboard },
  { path: '/accounts', component: Accounts },
  { path: '/status', component: Status },
]

export default createRouter({
  history: createWebHistory(),
  routes,
})
