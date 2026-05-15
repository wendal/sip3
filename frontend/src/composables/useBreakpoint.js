import { ref, onMounted, onUnmounted } from 'vue'

/**
 * Responsive breakpoint helper based on window.innerWidth.
 *  - mobile:  < 768
 *  - tablet:  768 - 991
 *  - desktop: >= 992
 */
export function useBreakpoint() {
  const width = ref(typeof window !== 'undefined' ? window.innerWidth : 1280)
  const isMobile = ref(false)
  const isTablet = ref(false)
  const isDesktop = ref(true)

  function update() {
    if (typeof window === 'undefined') return
    width.value = window.innerWidth
    isMobile.value = width.value < 768
    isTablet.value = width.value >= 768 && width.value < 992
    isDesktop.value = width.value >= 992
  }

  onMounted(() => {
    update()
    window.addEventListener('resize', update)
  })
  onUnmounted(() => {
    if (typeof window !== 'undefined') window.removeEventListener('resize', update)
  })

  return { width, isMobile, isTablet, isDesktop }
}
