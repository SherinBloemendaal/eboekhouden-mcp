<script lang="ts">
import { nextTick, onBeforeUnmount, onMounted, type Ref } from 'vue'

/** The open sheets and alerts, topmost last. Only the top one answers Esc and keeps Tab inside itself. */
const layers: HTMLElement[] = []

const FOCUSABLE = [
  'a[href]', 'button:not([disabled])', 'input:not([disabled]):not([type="hidden"])', 'select:not([disabled])',
  'textarea:not([disabled])', '[tabindex]:not([tabindex="-1"])',
].join(', ')

function focusables(panel: HTMLElement): HTMLElement[] {
  return [...panel.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => !el.closest('[inert], [hidden], [aria-hidden="true"]'))
}

/**
 * Makes `panel` a modal layer, shared by Sheet and Alert: Esc closes only the top layer, Tab and Shift+Tab cycle
 * inside it, `initialFocus` picks what gets the focus on open, and on close the focus goes back to where it was.
 */
export function useDialogLayer(panel: Ref<HTMLElement | null>, close: () => void, initialFocus: (panel: HTMLElement) => HTMLElement | null): void {
  let opened: HTMLElement | null = null
  let previous: HTMLElement | null = null

  function onKey(event: KeyboardEvent) {
    if (!opened || layers.at(-1) !== opened) return
    if (event.key === 'Escape') {
      event.preventDefault()
      close()
      return
    }
    if (event.key !== 'Tab') return
    const items = focusables(opened)
    const active = document.activeElement
    const inside = active instanceof HTMLElement && opened.contains(active)
    if (!items.length) {
      event.preventDefault()
      opened.focus()
      return
    }
    const first = items[0]!
    const last = items.at(-1)!
    if (event.shiftKey && (!inside || active === first || active === opened)) {
      event.preventDefault()
      last.focus()
    } else if (!event.shiftKey && (!inside || active === last)) {
      event.preventDefault()
      first.focus()
    }
  }

  onMounted(async () => {
    previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    opened = panel.value
    if (!opened) return
    layers.push(opened)
    document.addEventListener('keydown', onKey)
    await nextTick()
    if (opened && layers.includes(opened)) (initialFocus(opened) ?? opened).focus()
  })

  onBeforeUnmount(() => {
    document.removeEventListener('keydown', onKey)
    if (!opened) return
    const index = layers.indexOf(opened)
    if (index >= 0) layers.splice(index, 1)
    // Back to the button that opened it, when that is still on screen and not behind another layer.
    const top = layers.at(-1)
    if (previous?.isConnected && (!top || top.contains(previous))) previous.focus({ preventScroll: true })
    opened = null
    previous = null
  })
}
</script>

<script setup lang="ts">
// A macOS-style sheet: drops from the toolbar, glass, Esc or its buttons close it; a click beside it does not, as on
// macOS, so dragging the window by the dimmed area moves the window. Mount it inside <Transition name="modal">
// so it can leave: a v-if alone takes it out in one frame. Header and footer stay put; the body scrolls when the
// window is too low for it.
import { ref } from 'vue'

const props = defineProps<{ title: string; width?: number }>()
const emit = defineEmits<{ close: [] }>()
const panel = ref<HTMLElement | null>(null)

useDialogLayer(panel, () => emit('close'), (el) => el.querySelector<HTMLElement>('input:not([readonly]), select, textarea, button.primary'))
</script>

<template>
  <div class="scrim" data-tauri-drag-region>
    <div ref="panel" class="sheet" role="dialog" aria-modal="true" tabindex="-1" :aria-label="props.title" :style="{ width: `${props.width ?? 520}px` }">
      <h2>{{ props.title }}</h2>
      <div class="body"><slot /></div>
      <div class="foot"><slot name="footer" /></div>
    </div>
  </div>
</template>

<style scoped>
.scrim {
  position: fixed; inset: 0; z-index: 20; display: flex; justify-content: center; align-items: flex-start;
  padding-top: 52px; background: rgba(0, 0, 0, 0.1); animation: fade 150ms ease both;
}
.scrim.modal-leave-active { animation: fade-out 140ms cubic-bezier(0.4, 0, 1, 1) both; pointer-events: none; }
.modal-leave-active .sheet { animation: lift 140ms cubic-bezier(0.4, 0, 1, 1) both; }
/* 52 px from the top and 24 px free below: in a low window the footer with its buttons stays in view. */
.sheet {
  max-width: calc(100% - 48px); max-height: calc(100% - 52px - 24px); box-sizing: border-box; border-radius: 22px;
  display: flex; flex-direction: column; outline: none;
  background: var(--glass-pop); -webkit-backdrop-filter: blur(40px) saturate(190%); backdrop-filter: blur(40px) saturate(190%);
  box-shadow: var(--pop-shadow); animation: drop 260ms var(--ease) both; transform-origin: 50% 0;
}
h2 { flex: none; padding: 18px 20px 0; font-size: 15px; font-weight: 700; }
.body { flex: 1 1 auto; min-height: 0; overflow-y: auto; overscroll-behavior: contain; padding: 12px 20px 4px; display: flex; flex-direction: column; gap: 14px; }
/* The body scrolls instead of squeezing its content. */
.body > :deep(*) { flex-shrink: 0; }
.foot { flex: none; padding: 16px 20px 18px; display: flex; justify-content: flex-end; gap: 8px; }
@keyframes drop { from { opacity: 0; transform: translateY(-10px) scale(0.98); } to { opacity: 1; transform: none; } }
@keyframes lift { to { opacity: 0; transform: translateY(-8px) scale(0.98); } }
@keyframes fade { from { opacity: 0; } to { opacity: 1; } }
@keyframes fade-out { to { opacity: 0; } }
@media (prefers-reduced-motion: reduce) {
  .sheet { animation: fade 150ms ease both; }
  .modal-leave-active .sheet { animation: fade-out 140ms cubic-bezier(0.4, 0, 1, 1) both; }
}
</style>
