<script setup lang="ts">
// Instellingen (Cmd+,): appearance and when sessions end. Changes save at once, as in macOS settings.
import { onMounted, ref } from 'vue'
import Icon from '../components/Icon.vue'
import TraceView from '../components/TraceView.vue'
import { getSettings, saveSettings, signinTraces, type Appearance, type Settings, type Trace } from '../lib/accounts'
import { autostartGet, autostartSet, openLogDir } from '../lib/server'

const settings = ref<Settings | null>(null)
const error = ref<string | null>(null)
const autostart = ref(false)
const isMac = !navigator.userAgent.includes('Windows')

const traces = ref<Trace[]>([])
const openTrace = ref<number | null>(0)

onMounted(async () => {
  traces.value = await signinTraces().catch(() => [])
  settings.value = await getSettings()
  autostart.value = await autostartGet().catch(() => false)
})

async function toggleAutostart() {
  error.value = null
  try {
    autostart.value = await autostartSet(!autostart.value)
  } catch (e) {
    error.value = String(e)
  }
}

async function save(patch: Partial<Settings>) {
  if (!settings.value) return
  const next = { ...settings.value, ...patch }
  error.value = null
  try {
    await saveSettings(next)
    settings.value = next
  } catch (e) {
    error.value = String(e)
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
        <button type="button" role="switch" class="switch" :aria-checked="settings.showInDock" aria-label="Toon in Dock" @click="save({ showInDock: !settings.showInDock })"><span /></button>
      </div>
    </section>
    <h2 class="section">Weergave en sessies</h2>
    <section class="group card">
      <div class="row">
        <span class="text"><strong>Weergave</strong><small>Systeem volgt de instelling van je Mac.</small></span>
        <div class="seg" role="radiogroup" aria-label="Weergave">
          <button
            v-for="a in appearances" :key="a.value" type="button" role="radio" :aria-checked="settings.appearance === a.value"
            :class="{ on: settings.appearance === a.value }" @click="save({ appearance: a.value })"
          >{{ a.label }}</button>
        </div>
      </div>
      <div class="row">
        <span class="text"><strong>Uitloggen na stilte</strong><small>Stilte: geen aanroep van een AI-client voor dat account.</small></span>
        <select class="select" :value="settings.idleMinutes" aria-label="Uitloggen na stilte" @change="save({ idleMinutes: Number(($event.target as HTMLSelectElement).value) })">
          <option :value="15">15 minuten</option>
          <option :value="30">30 minuten</option>
          <option :value="60">1 uur</option>
          <option :value="240">4 uur</option>
        </select>
      </div>
      <p class="hint">Afsluiten vergeet altijd alle sessies; EB MCP bewaart ze nergens. Na een update log je zelf opnieuw in.</p>
      <p v-if="error" class="error" role="alert">{{ error }}</p>
    </section>
    <h2 class="section">Diagnose</h2>
    <section class="group card diagnose">
      <p class="hint first">De laatste aanmeldingen, stap voor stap: wat EB MCP op de pagina van e-Boekhouden zag en deed. Zonder wachtwoorden, codes of cookies, dus veilig om te delen.</p>
      <div v-for="(t, i) in traces" :key="t.startedAt + t.accountId" class="trace-row">
        <button type="button" class="trace-head" :aria-expanded="openTrace === i" @click="openTrace = openTrace === i ? null : i">
          <Icon name="list" :size="14" />
          <span class="trace-title">{{ t.label }}</span>
          <span class="trace-when">{{ new Date(t.startedAt).toLocaleString('nl-NL', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }) }}</span>
          <span class="trace-outcome" :class="{ ok: t.outcome?.startsWith('Done'), bad: t.outcome && !t.outcome.startsWith('Done') }">{{ t.outcome?.split(':')[0] ?? 'Loopt' }}</span>
        </button>
        <Transition name="accordion">
          <div v-if="openTrace === i" class="accordion"><div class="trace-body"><TraceView :trace="t" /></div></div>
        </Transition>
      </div>
      <p v-if="!traces.length" class="hint first">Nog geen aanmeldingen sinds EB MCP startte.</p>
      <div class="diag-actions">
        <button type="button" class="btn small" @click="signinTraces().then((t) => (traces = t))"><Icon name="refresh" :size="14" />Vernieuwen</button>
        <button type="button" class="btn small" @click="openLogDir"><Icon name="external" :size="14" />Open logmap</button>
      </div>
    </section>
  </div>
</template>

<style scoped>
.page { padding: 8px 28px 28px; max-width: 720px; }
.card { padding: 6px 18px 14px; display: flex; flex-direction: column; }
.row { display: flex; align-items: center; gap: 16px; min-height: 52px; border-bottom: 0.5px solid var(--line-soft); }
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
.card .row:last-child { border-bottom: 0; }
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
.trace-body { padding: 4px 6px 10px; }
.diag-actions { display: flex; gap: 8px; margin-top: 10px; }
.hint { margin-top: 12px; font-size: 12px; color: var(--muted); }
.error { margin-top: 8px; font-size: 12px; color: var(--danger); }
</style>
