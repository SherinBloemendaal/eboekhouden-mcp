<script setup lang="ts">
// A change an AI client asked for, waiting for the user's word. Only Goedkeuren lets it go to e-Boekhouden;
// Weigeren, Esc and closing the sheet all refuse it, so nothing ever changes by accident.
import { computed, onMounted, ref, watch } from 'vue'
import { connect, decideApproval, store } from '../lib/accounts'
import Avatar from './Avatar.vue'
import Icon from './Icon.vue'
import Sheet from './Sheet.vue'

onMounted(() => void connect())

/** The oldest waiting proposal: they are answered one at a time, in the order they came. */
const current = computed(() => store.approvals[0] ?? null)
const others = computed(() => Math.max(0, store.approvals.length - 1))
const busy = ref<'approve' | 'refuse' | null>(null)
const error = ref<string | null>(null)

watch(() => current.value?.id, () => {
  busy.value = null
  error.value = null
})

async function decide(approve: boolean) {
  const proposal = current.value
  if (!proposal || busy.value) return
  busy.value = approve ? 'approve' : 'refuse'
  error.value = null
  try {
    await decideApproval(proposal.id, approve)
  } catch (e) {
    error.value = String(e)
  } finally {
    busy.value = null
  }
}
</script>

<template>
  <Transition name="modal">
    <Sheet v-if="current" :title="current.title" :width="480" @close="decide(false)">
      <span class="who">
        <Avatar :label="current.account" :size="32" />
        <span class="stack">
          <span class="name">{{ current.account }}</span>
          <span class="sub">{{ current.client }} wil dit wijzigen in e-Boekhouden</span>
        </span>
      </span>
      <dl class="lines selectable">
        <template v-for="(line, i) in current.lines" :key="i">
          <dt>{{ line.label }}</dt>
          <dd>{{ line.value }}</dd>
        </template>
      </dl>
      <p v-if="current.danger" class="note warn" role="note"><Icon name="alert" :size="14" />Dit is niet terug te draaien vanuit EB MCP.</p>
      <p v-if="others" class="hint">Hierna {{ others === 1 ? 'wacht nog 1 wijziging' : `wachten nog ${others} wijzigingen` }} op je akkoord.</p>
      <p v-if="error" class="note bad rise" role="alert">{{ error }}</p>
      <template #footer>
        <button type="button" class="btn" :disabled="!!busy" :aria-busy="busy === 'refuse'" @click="decide(false)"><span class="btn-label">Weigeren</span></button>
        <button type="button" class="btn" :class="current.danger ? 'danger-fill' : 'primary'" :disabled="!!busy" :aria-busy="busy === 'approve'" @click="decide(true)">
          <span class="btn-label">Goedkeuren</span>
        </button>
      </template>
    </Sheet>
  </Transition>
</template>

<style scoped>
.who { display: flex; align-items: center; gap: 10px; }
.stack { display: flex; flex-direction: column; min-width: 0; }
.name { font-weight: 600; }
.sub { font-size: 12px; color: var(--muted); }
.lines { display: grid; grid-template-columns: minmax(0, auto) minmax(0, 1fr); column-gap: 14px; row-gap: 6px; margin: 14px 0 0; padding: 12px; border-radius: 10px; background: var(--sunken); }
dt { color: var(--label); font-size: 12px; line-height: 18px; white-space: nowrap; }
dd { margin: 0; line-height: 18px; overflow-wrap: anywhere; }
.note { display: flex; align-items: center; gap: 8px; margin: 10px 0 0; padding: 9px 11px; border-radius: 10px; font-size: 12px; line-height: 16px; }
.note.warn { background: var(--warning-soft, var(--danger-soft)); color: var(--warning, var(--danger)); }
.note.bad { background: var(--danger-soft); color: var(--danger); }
.hint { margin: 10px 0 0; font-size: 11px; color: var(--muted); }
</style>
