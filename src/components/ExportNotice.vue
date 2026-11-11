<script setup lang="ts">
// Where an export went, with Toon in Finder, or why it failed. The page's Exporteer button runs `state.run()`; the
// default slot says what the file holds and who may see it.
import type { ExportState } from '../lib/exporting'
import { revealName } from '../lib/server'
import Icon from './Icon.vue'

defineProps<{ state: ExportState }>()
</script>

<template>
  <Transition name="accordion">
    <div v-if="state.path" class="accordion">
      <div>
        <div class="export-slot">
          <div class="note ok export-note" role="status">
            <Icon name="check" :size="14" class="note-icon" />
            <p class="export-text">Export opgeslagen in <span class="mono selectable export-path">{{ state.path }}</span>. <slot /></p>
            <button type="button" class="btn small reveal" :disabled="state.revealing" :aria-busy="state.revealing" @click="state.reveal()"><span class="btn-label">{{ revealName }}</span></button>
            <button type="button" class="btn small ghost" @click="state.close()">Sluiten</button>
          </div>
          <Transition name="accordion">
            <div v-if="state.revealError" class="accordion"><div><p class="note bad reveal-error" role="alert">{{ state.revealError }}</p></div></div>
          </Transition>
        </div>
      </div>
    </div>
  </Transition>
  <Transition name="accordion">
    <div v-if="state.error" class="accordion">
      <div>
        <div class="export-slot">
          <div class="note bad export-note" role="alert">
            <Icon name="alert" :size="14" class="note-icon" />
            <p class="export-text selectable">{{ state.error }}</p>
            <button type="button" class="btn small ghost" @click="state.close()">Sluiten</button>
          </div>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.export-slot { padding: 2px 16px 10px; display: flex; flex-direction: column; gap: 6px; }
.note { padding: 9px 11px; border-radius: 10px; font-size: 12px; line-height: 16px; }
.note.bad { background: var(--danger-soft); color: var(--danger); }
.note.ok { background: var(--success-soft); color: var(--success); }
.note.export-note { display: flex; align-items: flex-start; gap: 8px; padding: 5px 6px 5px 11px; }
.note-icon { flex: none; margin-top: 5px; }
.export-text { flex: 1; min-width: 0; padding: 3px 0; }
.export-path { overflow-wrap: anywhere; }
.export-note .btn { flex: none; }
.export-note .btn.ghost { color: inherit; }
</style>
