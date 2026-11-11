<script setup lang="ts">
// The steps of a sign-in out of sight, and the code field when e-Boekhouden asks for one.
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { sendCode, signinTraces, type AccountView, type SigninEvent, type Trace } from '../lib/accounts'
import TraceView from './TraceView.vue'
import { openEboekhouden, unlockName } from '../lib/server'
import Icon from './Icon.vue'

const props = defineProps<{ account: AccountView; event: SigninEvent | undefined; compact?: boolean }>()

const ORDER = ['credentials', 'submit', 'code', 'administration'] as const
const code = ref('')
const codeError = ref<string | null>(null)
const sending = ref(false)
const lastIndex = ref(0)
/** Whether this sign-in asked for a two-step code: only then the list has that row. */
const codeSeen = ref(false)

function indexOf(step: string): number {
  // Sending the two-step code is still the code step.
  if (step === 'submit') return codeSeen.value ? 2 : 1
  const i = ORDER.indexOf(step as (typeof ORDER)[number])
  return i < 0 ? lastIndex.value : i
}

watch(
  () => props.event?.step,
  (step) => {
    // A sign-in starts by reading the credentials: whatever an earlier one asked no longer counts.
    if (step === 'credentials') codeSeen.value = false
    if (step === 'code') codeSeen.value = true
    if (step && !['done', 'failed', 'unknown', 'cancelled'].includes(step)) lastIndex.value = indexOf(step)
    if (step === 'code') {
      code.value = ''
      codeError.value = null
    }
  },
  { immediate: true },
)

const asked = computed(() => codeSeen.value)
const rows = computed(() => {
  const step = props.event?.step
  const labels = [
    props.account.store === 'onePassword' ? 'Gegevens uit 1Password lezen' : `${unlockName} voor het wachtwoord`,
    'Inloggen bij e-Boekhouden',
    ...(asked.value ? ['Tweestapsverificatie'] : []),
    'Administratie ophalen',
  ]
  const at = asked.value ? lastIndex.value : lastIndex.value >= 3 ? 2 : lastIndex.value
  return labels.map((label, i) => {
    // Without a known step nothing runs: no mark may spin for a sign-in this window knows nothing of.
    let state: 'done' | 'run' | 'wait' | 'fail' = !step ? 'wait' : i < at ? 'done' : i === at ? 'run' : 'wait'
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

// Debugging a sign-in: Details shows the trace, refreshed while it runs.
const running = computed(() => !!props.event && !['done', 'failed', 'unknown', 'cancelled'].includes(props.event.step))
const ended = computed(() => !!props.event && ['done', 'failed', 'unknown'].includes(props.event.step))
const showDetails = ref(false)
const trace = ref<Trace | null>(null)
let poll: ReturnType<typeof setInterval> | undefined

async function loadTrace() {
  trace.value = (await signinTraces()).find((t) => t.accountId === props.account.id) ?? null
}
watch(showDetails, (open) => {
  clearInterval(poll)
  if (!open) return
  void loadTrace()
  poll = setInterval(() => running.value && void loadTrace(), 1000)
})
watch(() => props.event?.step, () => {
  if (showDetails.value) void loadTrace()
})
onBeforeUnmount(() => {
  clearInterval(poll)
})

// An authenticator gives 6 digits; a backup code can hold letters too (EB MCP takes up to 12 characters).
// No maxlength on the field: a pasted "ABCD-EFGH-IJKL" would be cut before its dashes are stripped here.
const CODE_MAX = 12
function onInput() {
  code.value = code.value.replace(/[^0-9a-z]/gi, '').slice(0, CODE_MAX)
  if (/^\d{6}$/.test(code.value)) void submitCode()
}
</script>

<template>
  <div class="steps" :class="{ compact }">
    <TransitionGroup tag="ol" name="step" role="list">
      <li v-for="row in rows" :key="row.label" :class="row.state">
        <span class="mark" aria-hidden="true">
          <Transition name="mark">
            <span v-if="row.state === 'done'" key="done" class="disc ok"><Icon name="check" :size="11" /></span>
            <span v-else-if="row.state === 'run'" key="run" class="spin" />
            <span v-else-if="row.state === 'fail'" key="fail" class="disc bad">!</span>
          </Transition>
        </span>
        {{ row.label }}
      </li>
    </TransitionGroup>
    <Transition name="accordion">
      <div v-if="event?.step === 'code'" class="accordion">
        <div>
          <form class="code" @submit.prevent="submitCode">
            <label :for="`code-${account.id}`">Code uit je authenticator-app</label>
            <span class="code-row">
              <input
                :id="`code-${account.id}`" v-model="code" class="field mono" autocomplete="one-time-code" autocapitalize="off" spellcheck="false"
                placeholder="000000" @input="onInput"
              />
              <button type="submit" class="btn primary" :disabled="sending || code.length < 6" :aria-busy="sending"><span class="btn-label">Inloggen</span></button>
            </span>
            <span class="hint">Of een back-upcode. EB MCP geeft de code door aan e-Boekhouden en bewaart hem niet.</span>
            <span v-if="codeError || event?.message" class="error" role="alert">{{ codeError ?? event?.message }}</span>
          </form>
        </div>
      </div>
    </Transition>
    <Transition name="accordion">
      <div v-if="event?.message && (event.step === 'failed' || event.step === 'unknown')" class="accordion">
        <div>
          <p class="note bad" role="alert">{{ event.message }}</p>
          <button v-if="event.step === 'unknown'" type="button" class="btn small open-eb" @click="openEboekhouden"><Icon name="external" :size="14" />Open e-Boekhouden in je browser</button>
        </div>
      </div>
    </Transition>
    <div v-if="running || ended" class="debug">
      <button type="button" class="btn small ghost" :aria-expanded="showDetails" @click="showDetails = !showDetails">
        <Icon name="list" :size="14" />{{ showDetails ? 'Verberg details' : 'Details' }}
      </button>
    </div>
    <Transition name="accordion">
      <div v-if="showDetails" class="accordion">
        <div class="details"><TraceView v-if="trace" :trace="trace" /><p v-else class="hint">Nog niets vastgelegd.</p></div>
      </div>
    </Transition>
    <Transition name="accordion">
      <div v-if="event?.step === 'done'" class="accordion">
        <div>
          <p class="note ok" role="status">
            Ingelogd{{ event.message ? ` bij ${event.message}` : '' }}. De sessie staat alleen in het geheugen van EB MCP.
          </p>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.steps { display: flex; flex-direction: column; }
/* The gap sits inside each collapsing block, so it opens and closes with it. */
.code, .note { margin-top: 10px; }
/* Block flow, not flex: a leaving row is taken out of flow and must keep its own place. */
ol { position: relative; list-style: none; margin: 0; padding: 0; }
li + li { margin-top: 4px; }
li { display: flex; align-items: center; gap: 10px; min-height: 24px; color: var(--muted); transition: color 150ms ease; }
li.done, li.run { color: var(--text); }
li.fail { color: var(--danger); }
.compact li { min-height: 20px; font-size: 12px; }
.step-enter-active { transition: opacity 150ms var(--ease), transform 150ms var(--ease); }
.step-leave-active { position: absolute; transition: opacity 120ms cubic-bezier(0.4, 0, 1, 1); }
.step-enter-from { opacity: 0; transform: translateY(-4px); }
.step-leave-to { opacity: 0; }
.step-move { transition: transform 200ms var(--ease); }
.mark { position: relative; width: 16px; height: 16px; flex: none; font-size: 11px; font-weight: 700; }
.mark::before { content: ''; position: absolute; inset: 0; box-sizing: border-box; border-radius: 50%; border: 1.5px solid var(--off); transition: opacity 150ms ease; }
li:not(.wait) .mark::before { opacity: 0; }
.disc { position: absolute; inset: 0; border-radius: 50%; color: #fff; display: flex; align-items: center; justify-content: center; }
.disc.ok { background: var(--success-solid); }
.disc.bad { background: var(--danger); }
.spin { position: absolute; inset: 0; margin: auto; box-sizing: border-box; width: 12px; height: 12px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--muted); animation: spin 800ms linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
.mark-enter-active { transition: opacity 200ms var(--ease), transform 200ms var(--ease); }
.mark-leave-active { transition: opacity 150ms cubic-bezier(0.4, 0, 1, 1); }
.mark-enter-from, .mark-leave-to { opacity: 0; }
.disc.mark-enter-from { transform: scale(0.6); }
.code { display: flex; flex-direction: column; gap: 6px; }
.code label { font-size: 12px; font-weight: 600; color: var(--label); }
.code-row { display: flex; gap: 8px; }
.field { box-sizing: border-box; flex: 1; height: 32px; padding: 0 10px; border: 0; border-radius: 8px; background: var(--field-bg); box-shadow: inset 0 0 0 0.5px var(--field); font-size: 17px; letter-spacing: 6px; text-align: center; outline: none; transition: box-shadow 150ms ease; }
.field:focus { box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.hint { font-size: 11px; color: var(--muted); }
.error { font-size: 12px; color: var(--danger); }
.note { padding: 9px 11px; border-radius: 10px; font-size: 12px; line-height: 16px; }
.note.bad { background: var(--danger-soft); color: var(--danger); }
.open-eb { margin-top: 8px; }
.debug { display: flex; gap: 4px; margin-top: 8px; margin-left: -10px; }
.details { padding-top: 6px; }
.note.ok { background: var(--success-soft); color: var(--success); }
@media (prefers-reduced-motion: reduce) {
  .spin { animation-duration: 2400ms; }
  .step-enter-from, .disc.mark-enter-from { transform: none; }
  .step-move { transition: none; }
}
</style>
