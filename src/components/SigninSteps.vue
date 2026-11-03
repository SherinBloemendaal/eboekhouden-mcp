<script setup lang="ts">
// The steps of a sign-in out of sight, and the code field when e-Boekhouden asks for one.
import { computed, ref, watch } from 'vue'
import { sendCode, type AccountView, type SigninEvent } from '../lib/accounts'
import Icon from './Icon.vue'

const props = defineProps<{ account: AccountView; event: SigninEvent | undefined; compact?: boolean }>()

const ORDER = ['credentials', 'page', 'code', 'administration'] as const
const code = ref('')
const codeError = ref<string | null>(null)
const sending = ref(false)
const lastIndex = ref(0)

function indexOf(step: string): number {
  if (step === 'submit') return 1
  const i = ORDER.indexOf(step as (typeof ORDER)[number])
  return i < 0 ? lastIndex.value : i
}

watch(
  () => props.event?.step,
  (step) => {
    if (step && !['done', 'failed', 'unknown', 'cancelled'].includes(step)) lastIndex.value = indexOf(step)
    if (step === 'code') {
      code.value = ''
      codeError.value = null
    }
  },
  { immediate: true },
)

const asked = computed(() => lastIndex.value >= 2 || props.event?.step === 'code')
const rows = computed(() => {
  const step = props.event?.step
  const labels = [
    props.account.store === 'onePassword' ? 'Gegevens uit 1Password lezen' : 'Touch ID voor het wachtwoord',
    'Inloggen bij e-Boekhouden',
    ...(asked.value ? ['Tweestapsverificatie'] : []),
    'Administratie ophalen',
  ]
  const at = asked.value ? lastIndex.value : lastIndex.value >= 3 ? 2 : lastIndex.value
  return labels.map((label, i) => {
    let state: 'done' | 'run' | 'wait' | 'fail' = i < at ? 'done' : i === at ? 'run' : 'wait'
    if (step === 'done') state = 'done'
    if ((step === 'failed' || step === 'unknown') && i === at) state = 'fail'
    if (step === 'cancelled' && i >= at) state = 'wait'
    return { label, state }
  })
})

async function submitCode() {
  if (sending.value) return
  sending.value = true
  codeError.value = null
  try {
    await sendCode(props.account.id, code.value)
  } catch (error) {
    codeError.value = String(error)
  } finally {
    sending.value = false
  }
}

function onInput() {
  code.value = code.value.replace(/\D/g, '').slice(0, 8)
  if (code.value.length === 6) void submitCode()
}
</script>

<template>
  <div class="steps" :class="{ compact }">
    <ol role="list">
      <li v-for="row in rows" :key="row.label" :class="row.state" class="rise">
        <span class="mark" aria-hidden="true">
          <Icon v-if="row.state === 'done'" name="check" :size="11" />
          <span v-else-if="row.state === 'run'" class="spin" />
          <span v-else-if="row.state === 'fail'">!</span>
        </span>
        {{ row.label }}
      </li>
    </ol>
    <form v-if="event?.step === 'code'" class="code rise" @submit.prevent="submitCode">
      <label :for="`code-${account.id}`">Code uit je authenticator-app</label>
      <span class="code-row">
        <input
          :id="`code-${account.id}`" v-model="code" class="field mono" inputmode="numeric" autocomplete="one-time-code"
          maxlength="8" placeholder="000000" @input="onInput"
        />
        <button type="submit" class="btn primary" :disabled="sending || code.length < 6">{{ sending ? 'Bezig…' : 'Inloggen' }}</button>
      </span>
      <span class="hint">EB MCP geeft de code door aan e-Boekhouden en bewaart hem niet.</span>
      <span v-if="codeError" class="error" role="alert">{{ codeError }}</span>
    </form>
    <p v-if="event?.message && (event.step === 'failed' || event.step === 'unknown')" class="note bad rise" role="alert">{{ event.message }}</p>
    <p v-if="event?.step === 'done'" class="note ok rise" role="status">
      Ingelogd{{ event.message ? ` bij ${event.message}` : '' }}. De sessie staat alleen in het geheugen van EB MCP.
    </p>
  </div>
</template>

<style scoped>
.steps { display: flex; flex-direction: column; gap: 10px; }
ol { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
li { display: flex; align-items: center; gap: 10px; min-height: 24px; color: var(--muted); }
li.done, li.run { color: var(--text); }
li.fail { color: var(--danger); }
.compact li { min-height: 20px; font-size: 12px; }
.mark { width: 16px; height: 16px; flex: none; box-sizing: border-box; border-radius: 50%; border: 1.5px solid var(--off); display: flex; align-items: center; justify-content: center; font-size: 11px; font-weight: 700; }
li.done .mark { border: 0; background: var(--success-solid); color: #fff; }
li.fail .mark { border: 0; background: var(--danger); color: #fff; }
li.run .mark { border: 0; }
.spin { width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--muted); animation: spin 800ms linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
.code { display: flex; flex-direction: column; gap: 6px; }
.code label { font-size: 12px; font-weight: 600; color: var(--label); }
.code-row { display: flex; gap: 8px; }
.field { box-sizing: border-box; flex: 1; height: 32px; padding: 0 10px; border: 0; border-radius: 8px; background: var(--field-bg); box-shadow: inset 0 0 0 0.5px var(--field); font-size: 17px; letter-spacing: 6px; text-align: center; outline: none; transition: box-shadow 150ms ease; }
.field:focus { box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.hint { font-size: 11px; color: var(--muted); }
.error { font-size: 12px; color: var(--danger); }
.note { padding: 9px 11px; border-radius: 10px; font-size: 12px; line-height: 16px; }
.note.bad { background: var(--danger-soft); color: var(--danger); }
.note.ok { background: var(--success-soft); color: var(--success); }
@media (prefers-reduced-motion: reduce) { .spin { animation-duration: 2400ms; } }
</style>
