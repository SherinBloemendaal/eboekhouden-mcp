<script setup lang="ts">
import { computed } from 'vue'
import { initials } from '../lib/accounts'

const props = defineProps<{ label: string; size?: number; muted?: boolean }>()
const tones = [['#5e7ce2', '#3a56b8'], ['#4cc38a', '#2a9a63'], ['#f4b740', '#d48b12'], ['#b07be8', '#8650c9'], ['#f2685f', '#cf3a30']]
const tone = computed(() => tones[[...props.label].reduce((n, c) => n + c.charCodeAt(0), 0) % tones.length]!)
</script>

<template>
  <span
    class="avatar" :class="{ muted: props.muted }" aria-hidden="true"
    :style="{ width: `${props.size ?? 28}px`, height: `${props.size ?? 28}px`, fontSize: `${Math.round((props.size ?? 28) * 0.38)}px`, background: props.muted ? undefined : `linear-gradient(180deg, ${tone[0]}, ${tone[1]})` }"
  >{{ initials(props.label) }}</span>
</template>

<style scoped>
.avatar { flex: none; border-radius: 50%; color: #fff; font-weight: 600; display: inline-flex; align-items: center; justify-content: center; box-shadow: inset 0 0.5px 0 rgba(255, 255, 255, 0.35); }
.avatar.muted { background: var(--subtle); color: var(--muted); box-shadow: none; }
</style>
