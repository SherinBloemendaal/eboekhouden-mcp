<script setup lang="ts">
// The update notice in the sidebar (Windows) and the restart question (both): restarting signs everyone out.
import { computed, onMounted, ref } from 'vue'
import { store } from '../lib/accounts'
import { later, restartNow, startUpdates, updates } from '../lib/updates'
import Alert from './Alert.vue'

const showCard = ref(false)
onMounted(() => void startUpdates())

const signedIn = computed(() => store.accounts.filter((a) => a.session && !a.session.expired).map((a) => a.label))
const asking = computed(() => updates.restartAsked || showCard.value)

async function now() {
  await restartNow()
  showCard.value = false
}

async function notNow() {
  showCard.value = false
  await later()
}
</script>

<template>
  <button v-if="updates.available" type="button" class="update rise" @click="showCard = true">
    <span class="arrow" aria-hidden="true">↑</span>
    <span class="text"><strong>Update beschikbaar</strong><small>EB MCP {{ updates.available.version }}</small></span>
  </button>

  <Alert v-if="asking" :title="updates.restartAsked ? 'EB MCP herstarten om bij te werken?' : `EB MCP ${updates.available?.version} installeren?`" tone="warn" @close="notNow">
    <p v-if="!updates.restartAsked && updates.available?.notes" class="notes">{{ updates.available.notes }}</p>
    <p v-if="signedIn.length">{{ signedIn.join(' en ') }} {{ signedIn.length === 1 ? 'wordt' : 'worden' }} uitgelogd bij e-Boekhouden. Na de herstart log je opnieuw in; tot dan horen AI-clients dat ze even moeten wachten.</p>
    <p v-else>EB MCP start daarna opnieuw. Lopende aanroepen maken eerst af.</p>
    <p v-if="updates.progress !== null">Downloaden… {{ Math.round(updates.progress * 100) }}%</p>
    <p v-if="updates.error" class="error" role="alert">{{ updates.error }}</p>
    <template #buttons>
      <button type="button" class="btn" :disabled="updates.restarting" @click="notNow">{{ updates.channel === 'sparkle' ? 'Bij afsluiten' : 'Later' }}</button>
      <button type="button" class="btn primary" :disabled="updates.restarting" @click="now">{{ updates.restarting ? 'Bezig…' : 'Nu herstarten' }}</button>
    </template>
  </Alert>
</template>

<style scoped>
.update {
  display: flex; align-items: center; gap: 8px; width: 100%; box-sizing: border-box; height: 34px; padding: 0 10px; border: 0; border-radius: 10px; text-align: left;
  background: var(--glass-bar); box-shadow: var(--glass-bar-edge); transition: background-color 150ms ease;
}
.update:hover { background: var(--hover); }
.arrow { width: 18px; height: 18px; flex: none; border-radius: 50%; background: var(--accent); color: #fff; display: flex; align-items: center; justify-content: center; font-size: 11px; font-weight: 700; }
.text { display: flex; flex-direction: column; font-size: 12px; min-width: 0; }
.text small { font-size: 11px; color: var(--muted); }
.notes { white-space: pre-line; text-align: left; max-height: 140px; overflow: auto; }
.error { color: var(--danger); }
</style>
