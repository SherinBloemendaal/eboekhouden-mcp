<script setup lang="ts">
// A macOS alert: app icon, short title, text and equal buttons. Esc cancels. Mount it inside
// <Transition name="modal"> so it can leave: a v-if alone takes it out in one frame.
import { onBeforeUnmount, onMounted } from 'vue'

const props = defineProps<{ title: string; tone?: 'danger' | 'warn' }>()
const emit = defineEmits<{ close: [] }>()
const onKey = (event: KeyboardEvent) => event.key === 'Escape' && emit('close')
onMounted(() => document.addEventListener('keydown', onKey))
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))
</script>

<template>
  <div class="scrim" data-tauri-drag-region>
    <div class="alert" role="alertdialog" aria-modal="true" :aria-label="props.title">
      <span class="icon">
        <img src="/app-icon.png" alt="" width="56" height="56" />
        <span v-if="props.tone" class="badge" :class="props.tone">!</span>
      </span>
      <h2>{{ props.title }}</h2>
      <div class="text"><slot /></div>
      <div class="buttons"><slot name="buttons" /></div>
    </div>
  </div>
</template>

<style scoped>
.scrim { position: fixed; inset: 0; z-index: 30; display: flex; align-items: center; justify-content: center; background: rgba(0, 0, 0, 0.1); animation: fade 150ms ease both; }
.scrim.modal-leave-active { animation: fade-out 140ms cubic-bezier(0.4, 0, 1, 1) both; pointer-events: none; }
.modal-leave-active .alert { animation: shrink 120ms cubic-bezier(0.4, 0, 1, 1) both; }
.alert {
  width: 300px; box-sizing: border-box; padding: 20px 16px 16px; border-radius: 22px; display: flex; flex-direction: column; align-items: center; gap: 10px; text-align: center;
  background: var(--glass-pop); -webkit-backdrop-filter: blur(40px) saturate(190%); backdrop-filter: blur(40px) saturate(190%); box-shadow: var(--pop-shadow);
  animation: show 240ms var(--ease) both;
}
.icon { position: relative; display: inline-flex; }
.icon img { border-radius: 13px; }
.badge { position: absolute; right: -4px; bottom: -4px; width: 22px; height: 22px; border-radius: 50%; color: #fff; font-weight: 700; font-size: 13px; display: flex; align-items: center; justify-content: center; }
.badge.danger { background: #d93025; }
.badge.warn { background: #ff9f0a; }
h2 { font-size: 13px; font-weight: 700; }
.text { font-size: 11px; line-height: 15px; display: flex; flex-direction: column; gap: 8px; width: 100%; }
.buttons { width: 100%; margin-top: 6px; display: flex; gap: 8px; }
.buttons :deep(.btn) { flex: 1; }
@keyframes show { from { opacity: 0; transform: scale(0.96); } to { opacity: 1; transform: none; } }
@keyframes shrink { to { opacity: 0; transform: scale(0.97); } }
@keyframes fade { from { opacity: 0; } to { opacity: 1; } }
@keyframes fade-out { to { opacity: 0; } }
@media (prefers-reduced-motion: reduce) {
  .alert { animation: fade 150ms ease both; }
  .modal-leave-active .alert { animation: fade-out 120ms cubic-bezier(0.4, 0, 1, 1) both; }
}
</style>
