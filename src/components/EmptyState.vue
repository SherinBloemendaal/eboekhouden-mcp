<script setup lang="ts">
import Icon from './Icon.vue'

defineProps<{ icon: string; title: string }>()
</script>

<template>
  <div class="empty rise">
    <span class="badge"><Icon :name="icon" :size="24" /></span>
    <h2>{{ title }}</h2>
    <p><slot /></p>
  </div>
</template>

<style scoped>
.empty { height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; text-align: center; padding: 24px; box-sizing: border-box; }
.badge { width: 52px; height: 52px; border-radius: 50%; background: var(--subtle); color: var(--muted); display: flex; align-items: center; justify-content: center; }
h2 { font-size: 15px; font-weight: 700; }
p { max-width: 420px; color: var(--muted); line-height: 1.5; }
</style>
