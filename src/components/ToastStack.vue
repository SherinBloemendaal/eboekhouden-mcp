<script setup lang="ts">
// Short in-app notices in the top right of the positioned box it is placed in, newest on top. Each leaves by itself after `duration` ms;
// the clock stops while the pointer or the keyboard focus is on the stack, so nothing leaves while being read.
// At most `max` show, the rest as "en N meer". Screen readers hear new ones politely: the live region stays in
// the page, only its items come and go.
import { onBeforeUnmount, ref, watch } from 'vue'
import Icon from './Icon.vue'

export interface Toast {
  id: number | string
  title: string
  text?: string
  /** The label of the toast's one button, if it has one. */
  action?: string
  icon?: string
}

const props = withDefaults(defineProps<{ toasts: Toast[]; duration?: number; max?: number; label?: string }>(), {
  duration: 8000,
  max: 3,
  label: 'Meldingen',
})
const emit = defineEmits<{ close: [id: Toast['id']]; action: [id: Toast['id']] }>()

interface Clock { remaining: number; startedAt: number; handle: ReturnType<typeof setTimeout> | null }
const clocks = new Map<Toast['id'], Clock>()
const hovered = ref(false)
const focused = ref(false)
const paused = () => hovered.value || focused.value

function start(id: Toast['id'], clock: Clock) {
  clock.startedAt = Date.now()
  clock.handle = setTimeout(() => {
    clocks.delete(id)
    emit('close', id)
  }, clock.remaining)
}

function stop(clock: Clock) {
  if (clock.handle === null) return
  clearTimeout(clock.handle)
  clock.handle = null
  clock.remaining = Math.max(0, clock.remaining - (Date.now() - clock.startedAt))
}

watch(
  () => props.toasts.map((t) => t.id),
  (ids) => {
    for (const [id, clock] of clocks) {
      if (ids.includes(id)) continue
      stop(clock)
      clocks.delete(id)
    }
    for (const id of ids) {
      if (clocks.has(id)) continue
      const clock: Clock = { remaining: props.duration, startedAt: 0, handle: null }
      clocks.set(id, clock)
      if (!paused()) start(id, clock)
    }
  },
  { immediate: true },
)

watch(paused, (now) => {
  for (const [id, clock] of clocks) {
    if (now) stop(clock)
    else if (clock.handle === null) start(id, clock)
  }
})

onBeforeUnmount(() => {
  for (const clock of clocks.values()) stop(clock)
  clocks.clear()
})

function onFocusOut(event: FocusEvent) {
  const next = event.relatedTarget
  if (!(next instanceof Node) || !(event.currentTarget as HTMLElement).contains(next)) focused.value = false
}
</script>

<template>
  <section
    class="toasts" :aria-label="props.label" aria-live="polite" aria-atomic="false" aria-relevant="additions text"
    @mouseenter="hovered = true" @mouseleave="hovered = false" @focusin="focused = true" @focusout="onFocusOut"
  >
    <TransitionGroup name="toast" tag="ol" class="stack">
      <li v-for="toast in props.toasts.slice(0, props.max)" :key="toast.id" class="toast">
        <Icon :name="toast.icon ?? 'alert'" :size="16" class="toast-icon" />
        <div class="toast-body">
          <strong class="toast-title">{{ toast.title }}</strong>
          <span v-if="toast.text" class="toast-text">{{ toast.text }}</span>
          <button v-if="toast.action" type="button" class="btn small toast-action" @click="emit('action', toast.id)">{{ toast.action }}</button>
        </div>
        <button type="button" class="close" aria-label="Melding sluiten" title="Sluiten" @click="emit('close', toast.id)"><Icon name="x" :size="13" /></button>
      </li>
      <li v-if="props.toasts.length > props.max" key="more" class="more">en {{ props.toasts.length - props.max }} meer</li>
    </TransitionGroup>
  </section>
</template>

<style scoped>
/* Over the page in the parent's top right corner; alerts and sheets (fixed, z-index 30) still cover it. */
.toasts { position: absolute; top: 8px; right: 16px; z-index: 20; width: 320px; max-width: calc(100% - 32px); pointer-events: none; }
.stack { position: relative; margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 8px; }
.toast {
  box-sizing: border-box; display: flex; align-items: flex-start; gap: 10px; padding: 10px 8px 10px 12px; border-radius: 14px;
  background: var(--glass-pop); box-shadow: var(--pop-shadow); pointer-events: auto;
  -webkit-backdrop-filter: blur(40px) saturate(190%); backdrop-filter: blur(40px) saturate(190%);
}
.toast-icon { flex: none; margin-top: 1px; color: var(--accent); }
.toast-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.toast-title { font-size: 12px; line-height: 16px; overflow-wrap: anywhere; }
.toast-text { font-size: 11px; line-height: 15px; color: var(--muted); overflow-wrap: anywhere; }
.toast-action { align-self: flex-start; margin-top: 6px; }
.close {
  flex: none; width: 22px; height: 22px; padding: 0; border: 0; border-radius: 7px; background: transparent; color: var(--muted);
  display: inline-flex; align-items: center; justify-content: center;
  transition: background-color 150ms ease, color 150ms ease, transform 120ms var(--ease);
}
.close:hover { background: var(--hover); color: var(--text); }
.close:active { transform: scale(0.92); }
.close:focus-visible { outline: none; box-shadow: 0 0 0 3px rgba(10, 132, 255, 0.42); }
.more {
  align-self: flex-end; padding: 3px 10px; border-radius: 999px; font-size: 11px; color: var(--label);
  background: var(--glass-pop); box-shadow: var(--pop-shadow); pointer-events: auto;
}
@media (prefers-reduced-motion: reduce) {
  .close:active { transform: none; }
}
</style>
