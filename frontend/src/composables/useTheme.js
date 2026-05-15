import { ref, watchEffect } from 'vue'

const STORAGE_KEY = 'sip3-theme'
const root = typeof document !== 'undefined' ? document.documentElement : null

function detectInitial() {
  if (typeof window === 'undefined') return 'light'
  try {
    const saved = window.localStorage.getItem(STORAGE_KEY)
    if (saved === 'light' || saved === 'dark') return saved
  } catch { /* ignore */ }
  if (window.matchMedia?.('(prefers-color-scheme: dark)').matches) return 'dark'
  return 'light'
}

const theme = ref(detectInitial())

function apply(value) {
  if (!root) return
  if (value === 'dark') root.classList.add('dark')
  else root.classList.remove('dark')
}

watchEffect(() => {
  apply(theme.value)
  try { window.localStorage.setItem(STORAGE_KEY, theme.value) } catch { /* ignore */ }
})

export function useTheme() {
  function toggle() {
    theme.value = theme.value === 'dark' ? 'light' : 'dark'
  }
  function set(value) {
    if (value === 'light' || value === 'dark') theme.value = value
  }
  return { theme, toggle, set }
}
