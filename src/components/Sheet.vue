<script setup lang="ts">
// A macOS-style sheet: drops from the toolbar, glass, Esc closes.
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue'

const props = defineProps<{ title: string; width?: number }>()
const emit = defineEmits<{ close: [] }>()
const panel = ref<HTMLElement | null>(null)

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') emit('close')
}

onMounted(async () => {
  document.addEventListener('keydown', onKey)
  await nextTick()
  panel.value?.querySelector<HTMLElement>('input:not([readonly]), select, textarea, button.primary')?.focus()
})
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))
</script>

<template>
  <div class="scrim" @mousedown.self="emit('close')">
    <div ref="panel" class="sheet" role="dialog" aria-modal="true" :aria-label="props.title" :style="{ width: `${props.width ?? 520}px` }">
      <h2>{{ props.title }}</h2>
      <div class="body"><slot /></div>
      <div class="foot"><slot name="footer" /></div>
    </div>
  </div>
</template>

<style scoped>
.scrim {
  position: fixed; inset: 0; z-index: 20; display: flex; justify-content: center; align-items: flex-start;
  padding-top: 52px; background: rgba(0, 0, 0, 0.1); animation: fade 180ms ease both;
}
.sheet {
  max-width: calc(100% - 48px); box-sizing: border-box; border-radius: 22px; display: flex; flex-direction: column;
  background: var(--glass-pop); -webkit-backdrop-filter: blur(40px) saturate(190%); backdrop-filter: blur(40px) saturate(190%);
  box-shadow: var(--pop-shadow); animation: drop 260ms var(--ease) both; transform-origin: 50% 0;
}
h2 { padding: 18px 20px 0; font-size: 15px; font-weight: 700; }
.body { padding: 12px 20px 4px; display: flex; flex-direction: column; gap: 14px; }
.foot { padding: 16px 20px 18px; display: flex; justify-content: flex-end; gap: 8px; }
@keyframes drop { from { opacity: 0; transform: translateY(-10px) scale(0.98); filter: blur(4px); } to { opacity: 1; transform: none; filter: none; } }
@keyframes fade { from { opacity: 0; } to { opacity: 1; } }
@media (prefers-reduced-motion: reduce) { .sheet { animation: fade 150ms ease both; } }
</style>
