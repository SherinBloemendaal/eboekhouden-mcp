<script setup lang="ts">
// Instellingen (Cmd+,): appearance and when sessions end. Changes save at once, as in macOS settings.
import { onMounted, ref } from 'vue'
import { getSettings, saveSettings, type Appearance, type Settings } from '../lib/accounts'

const settings = ref<Settings | null>(null)
const error = ref<string | null>(null)

onMounted(async () => {
  settings.value = await getSettings()
})

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
      <p class="hint">Afsluiten logt altijd alle sessies uit bij e-Boekhouden. Na een update log je zelf opnieuw in.</p>
      <p v-if="error" class="error" role="alert">{{ error }}</p>
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
.hint { margin-top: 12px; font-size: 12px; color: var(--muted); }
.error { margin-top: 8px; font-size: 12px; color: var(--danger); }
</style>
