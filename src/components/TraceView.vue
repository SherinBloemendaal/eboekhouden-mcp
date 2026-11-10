<script setup lang="ts">
// A sign-in's diagnosis: each step EB MCP saw and did, with the seconds since the start, and a copy button.
import { ref } from 'vue'
import { traceText, type Trace } from '../lib/accounts'
import Icon from './Icon.vue'

const props = defineProps<{ trace: Trace }>()
const copied = ref(false)

async function copy() {
  await navigator.clipboard.writeText(traceText(props.trace))
  copied.value = true
  setTimeout(() => (copied.value = false), 1500)
}
</script>

<template>
  <div class="trace">
    <ol class="lines mono selectable">
      <li v-for="(line, i) in trace.lines" :key="i"><span class="ms">{{ (line.ms / 1000).toFixed(1) }} s</span><span>{{ line.text }}</span></li>
    </ol>
    <button type="button" class="btn small" @click="copy">
      <Icon name="copy" :size="14" />
      <span class="swap"><span :class="{ off: copied }" :aria-hidden="copied">Kopieer diagnose</span><span :class="{ off: !copied }" :aria-hidden="!copied">Gekopieerd</span></span>
    </button>
  </div>
</template>

<style scoped>
.trace { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
.lines { box-sizing: border-box; width: 100%; max-height: 200px; overflow: auto; margin: 0; padding: 8px 10px; list-style: none; border-radius: 10px; background: var(--sunken); font-size: 11px; line-height: 16px; }
.lines li { display: flex; gap: 10px; }
.lines li > span:last-child { min-width: 0; overflow-wrap: anywhere; }
.ms { flex: none; width: 44px; text-align: right; color: var(--muted); }
</style>
