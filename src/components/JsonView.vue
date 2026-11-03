<script setup lang="ts">
// Pretty JSON with quiet colours. Text is escaped before it is coloured.
import { computed } from 'vue'

const props = defineProps<{ value: unknown; maxHeight?: number }>()

function escape(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

const html = computed(() => {
  const text = escape(JSON.stringify(props.value, null, 2) ?? 'null')
  return text.replace(
    /("(?:\\.|[^"\\])*")(\s*:)?|\b(true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g,
    (match, str: string | undefined, colon: string | undefined, lit: string | undefined) => {
      if (str) {
        if (colon) return `<span class="k">${str}</span>${colon}`
        return str.startsWith('"…') ? `<span class="m">${str}</span>` : `<span class="s">${str}</span>`
      }
      if (lit) return `<span class="l">${lit}</span>`
      return `<span class="n">${match}</span>`
    },
  )
})
</script>

<template>
  <!-- eslint-disable-next-line vue/no-v-html -->
  <pre class="json selectable" :style="{ maxHeight: props.maxHeight ? `${props.maxHeight}px` : undefined }" v-html="html" />
</template>

<style scoped>
.json { margin: 0; padding: 10px 12px; border-radius: 10px; background: var(--sunken); font: 12px/18px var(--mono); overflow: auto; white-space: pre; }
.json :deep(.k) { color: #3e6d74; }
.json :deep(.s) { color: #c41a16; }
.json :deep(.n) { color: #1c00cf; }
.json :deep(.l) { color: #9b2393; }
.json :deep(.m) { color: var(--muted); }
@media (prefers-color-scheme: dark) {
  .json :deep(.k) { color: #67b7a4; }
  .json :deep(.s) { color: #fc6a5d; }
  .json :deep(.n) { color: #d0bf69; }
  .json :deep(.l) { color: #fc5fa3; }
}
</style>
