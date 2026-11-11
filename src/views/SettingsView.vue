<script setup lang="ts">
// Instellingen (Cmd+,): appearance and when sessions end. Changes save at once, as in macOS settings.
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import TraceView from '../components/TraceView.vue'
import {
  connect, getSettings, outcomeLabel, saveSettings, signinTraces, store, type Appearance, type Settings, type Trace,
} from '../lib/accounts'
import { autostartGet, autostartSet, isMac, openLogDir } from '../lib/server'

type Card = 'general' | 'view'

const settings = ref<Settings | null>(null)
/** Each card shows its own errors. */
const errors = ref<Record<Card, string | null>>({ general: null, view: null })
const autostart = ref(false)

const traces = ref<Trace[]>([])
const openKey = ref<string | null>(null)
const traceKey = (t: Trace) => `${t.startedAt}:${t.accountId}`

// While a shown sign-in has no outcome yet, the list reloads every 2 s; it stops when all have one or the page closes.
let poll: ReturnType<typeof setTimeout> | undefined
let mounted = false
let firstLoad = true

async function loadTraces() {
  clearTimeout(poll)
  const next = await signinTraces().catch(() => traces.value)
  if (!mounted) return
  traces.value = next
  // The newest one opens the first time, as before; after that the user's choice stays, also when new ones arrive.
  if (firstLoad && next[0]) openKey.value = traceKey(next[0])
  firstLoad = false
  clearTimeout(poll)
  if (next.some((t) => t.outcome === null)) poll = setTimeout(() => void loadTraces(), 2000)
}

// A sign-in that starts or moves on (in this window or the menu-bar panel) shows up without Vernieuwen.
const signinSteps = computed(() => Object.values(store.signins).map((e) => `${e.accountId}:${e.step}`).join(','))
watch(signinSteps, () => void loadTraces())

onMounted(async () => {
  mounted = true
  void connect()
  await loadTraces()
  settings.value = await getSettings()
  autostart.value = await autostartGet().catch(() => false)
})
onBeforeUnmount(() => {
  mounted = false
  clearTimeout(poll)
})

async function toggleAutostart() {
  errors.value.general = null
  try {
    autostart.value = await autostartSet(!autostart.value)
  } catch (e) {
    errors.value.general = String(e)
  }
}

async function save(patch: Partial<Settings>, card: Card) {
  if (!settings.value) return
  const next = { ...settings.value, ...patch }
  errors.value[card] = null
  try {
    await saveSettings(next)
    settings.value = next
  } catch (e) {
    errors.value[card] = String(e)
  }
}

const appearances: { value: Appearance; label: string }[] = [
  { value: 'system', label: 'Systeem' },
  { value: 'light', label: 'Licht' },
  { value: 'dark', label: 'Donker' },
]
</script>

<template>
  <div v-if="settings" class="page">
    <h2 class="section">Algemeen</h2>
    <section class="group card">
      <div class="row">
        <span class="text"><strong>Openen bij inloggen</strong><small>EB MCP start dan op de achtergrond, alleen in de menubalk.</small></span>
        <button type="button" role="switch" class="switch" :aria-checked="autostart" aria-label="Openen bij inloggen" @click="toggleAutostart"><span /></button>
      </div>
      <div v-if="isMac" class="row">
        <span class="text"><strong>Toon in Dock</strong><small>Uit: alleen in de menubalk. Sluit je het venster, dan blijft EB MCP altijd draaien.</small></span>
        <button type="button" role="switch" class="switch" :aria-checked="settings.showInDock" aria-label="Toon in Dock" @click="save({ showInDock: !settings.showInDock }, 'general')"><span /></button>
      </div>
      <Transition name="accordion">
        <div v-if="errors.general" class="accordion"><div><p class="error" role="alert">{{ errors.general }}</p></div></div>
      </Transition>
    </section>
    <h2 class="section">Weergave en sessies</h2>
    <section class="group card">
      <div class="row">
        <span class="text"><strong>Weergave</strong><small>Systeem volgt de instelling van je {{ isMac ? 'Mac' : 'computer' }}.</small></span>
        <div class="seg" role="radiogroup" aria-label="Weergave">
          <button
            v-for="a in appearances" :key="a.value" type="button" role="radio" :aria-checked="settings.appearance === a.value"
            :class="{ on: settings.appearance === a.value }" @click="save({ appearance: a.value }, 'view')"
          >{{ a.label }}</button>
        </div>
      </div>
      <div class="row">
        <span class="text"><strong>Uitloggen na stilte</strong><small>Stilte: geen aanroep van een AI-client voor dat account.</small></span>
        <select class="select" :value="settings.idleMinutes" aria-label="Uitloggen na stilte" @change="save({ idleMinutes: Number(($event.target as HTMLSelectElement).value) }, 'view')">
          <option :value="15">15 minuten</option>
          <option :value="30">30 minuten</option>
          <option :value="60">1 uur</option>
          <option :value="240">4 uur</option>
        </select>
      </div>
      <p class="hint">Afsluiten vergeet altijd alle sessies; EB MCP bewaart ze nergens. Na een update log je zelf opnieuw in.</p>
      <Transition name="accordion">
        <div v-if="errors.view" class="accordion"><div><p class="error" role="alert">{{ errors.view }}</p></div></div>
      </Transition>
    </section>
    <h2 class="section">Diagnose</h2>
    <section class="group card diagnose">
      <p class="hint first">De laatste aanmeldingen, stap voor stap: wat EB MCP naar e-Boekhouden stuurde en terugkreeg. Zonder wachtwoorden, codes of tokens, dus veilig om te delen.</p>
      <div v-for="t in traces" :key="traceKey(t)" class="trace-row">
        <button type="button" class="trace-head" :aria-expanded="openKey === traceKey(t)" @click="openKey = openKey === traceKey(t) ? null : traceKey(t)">
          <Icon name="list" :size="14" />
          <span class="trace-title">{{ t.label }}</span>
          <span class="trace-when">{{ new Date(t.startedAt).toLocaleString('nl-NL', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }) }}</span>
          <span class="trace-outcome" :class="outcomeLabel(t.outcome).tone">{{ outcomeLabel(t.outcome).text }}</span>
        </button>
        <Transition name="accordion">
          <div v-if="openKey === traceKey(t)" class="accordion"><div class="trace-body"><TraceView :trace="t" /></div></div>
        </Transition>
      </div>
      <p v-if="!traces.length" class="hint first">Nog geen aanmeldingen sinds EB MCP startte.</p>
      <div class="diag-actions">
        <button type="button" class="btn small" @click="loadTraces"><Icon name="refresh" :size="14" />Vernieuwen</button>
        <button type="button" class="btn small" @click="openLogDir"><Icon name="external" :size="14" />Open logmap</button>
      </div>
    </section>
  </div>
</template>

<style scoped>
.page { padding: 8px 28px 28px; max-width: 720px; }
.card { padding: 6px 18px 14px; display: flex; flex-direction: column; }
/* Lines sit between rows (and above the hint), so an error below the last row adds no line of its own. */
.row { display: flex; align-items: center; gap: 16px; min-height: 52px; }
.row + .row, .row + .hint { border-top: 0.5px solid var(--line-soft); }
.text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
.text small { font-size: 11px; color: var(--muted); }
.seg { display: inline-flex; padding: 2px; border-radius: 999px; background: var(--subtle); }
.seg button { height: 24px; padding: 0 12px; border: 0; border-radius: 999px; background: transparent; font-size: 12px; transition: background-color 180ms var(--ease), box-shadow 180ms var(--ease); }
.seg button:hover { background: var(--hover); }
.seg button.on { background: var(--seg-on); box-shadow: var(--seg-on-shadow); font-weight: 500; }
.select { height: 24px; padding: 0 8px; border: 0; border-radius: 7px; background: var(--glass-bar); box-shadow: var(--glass-bar-edge); }
.section { margin: 18px 4px 8px; font-size: 12px; font-weight: 600; color: var(--muted); }
.section:first-child { margin-top: 0; }
.group + .section { margin-top: 22px; }
.switch { position: relative; width: 38px; height: 22px; flex: none; padding: 0; border: 0; border-radius: 999px; background: var(--subtle); box-shadow: inset 0 0 0 0.5px var(--line); transition: background-color 200ms var(--ease), filter 150ms ease; }
.switch span { position: absolute; top: 2px; left: 2px; width: 18px; height: 18px; border-radius: 50%; background: #fff; box-shadow: 0 1px 3px rgba(0, 0, 0, 0.25); transition: transform 200ms var(--ease); }
.switch[aria-checked='true'] { background: var(--accent); }
.switch[aria-checked='true'] span { transform: translateX(16px); }
.switch:hover { filter: brightness(0.97); }
.switch:focus-visible { outline: none; box-shadow: 0 0 0 3px rgba(10, 132, 255, 0.42); }
@media (prefers-reduced-motion: reduce) { .switch, .switch span { transition-duration: 1ms; } }
.diagnose { padding-top: 12px; gap: 6px; }
.hint.first { margin-top: 0; margin-bottom: 6px; }
.trace-row { border-bottom: 0.5px solid var(--line-soft); }
.trace-head { box-sizing: border-box; width: 100%; display: flex; align-items: center; gap: 8px; min-height: 34px; padding: 0 6px; border: 0; border-radius: 8px; background: transparent; text-align: left; transition: background-color 150ms ease; }
.trace-head:hover { background: var(--hover); }
.trace-title { flex: 1; min-width: 0; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.trace-when { font-size: 11px; color: var(--muted); }
.trace-outcome { font-size: 11px; font-weight: 600; color: var(--muted); }
.trace-outcome.ok { color: var(--success); }
.trace-outcome.bad { color: var(--danger); }
.trace-outcome.run { color: var(--accent); }
.trace-body { padding: 4px 6px 10px; }
.diag-actions { display: flex; gap: 8px; margin-top: 10px; }
.hint { margin-top: 12px; font-size: 12px; color: var(--muted); }
.row + .hint { margin-top: 0; padding-top: 12px; }
/* Padding, not margin: the accordion's clipping child must not let a margin collapse through it. */
.error { margin: 0; padding-top: 8px; font-size: 12px; color: var(--danger); }
</style>
