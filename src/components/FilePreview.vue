<script setup lang="ts">
// The file a waiting proposal sends along (archief_uploaden), shown on Akkoord before Goedkeuren: an image as is,
// a PDF in the webview's own viewer, anything else by name with Openen. The app reads only the proposal's own file.
import { onBeforeUnmount, ref, watch } from 'vue'
import Icon from './Icon.vue'
import { approvalFile, openApprovalFile, previewKind, type Attachment } from '../lib/approvals'
import { size } from '../lib/log'
import { inApp, revealName } from '../lib/server'

const props = defineProps<{ id: number; file: Attachment }>()

const url = ref<string | null>(null)
const changed = ref(false)
const error = ref<string | null>(null)
const loading = ref(false)
const shown = () => previewKind(props.file.kind)

function clear() {
  if (url.value?.startsWith('blob:')) URL.revokeObjectURL(url.value)
  url.value = null
}

async function load() {
  clear()
  changed.value = false
  error.value = null
  const kind = shown()
  if (!kind || !inApp()) return
  const id = props.id
  loading.value = true
  try {
    const preview = await approvalFile(id)
    if (id !== props.id) return
    changed.value = preview.changed
    const type = preview.kind ?? 'application/octet-stream'
    if (kind === 'image') {
      url.value = `data:${type};base64,${preview.data}`
    } else {
      // A PDF goes into a frame as a blob: the webview shows it with its own viewer.
      const bytes = Uint8Array.from(atob(preview.data), (c) => c.charCodeAt(0))
      url.value = URL.createObjectURL(new Blob([bytes], { type }))
    }
  } catch (e) {
    if (id === props.id) error.value = String(e)
  } finally {
    if (id === props.id) loading.value = false
  }
}

async function open(reveal: boolean) {
  error.value = null
  try {
    await openApprovalFile(props.id, reveal)
  } catch (e) {
    error.value = String(e)
  }
}

watch(() => [props.id, props.file.path], () => void load(), { immediate: true })
onBeforeUnmount(clear)
</script>

<template>
  <div class="file-preview">
    <div class="file-head">
      <Icon name="list" :size="14" />
      <span class="file-name selectable" :title="file.path">{{ file.name }}</span>
      <span class="file-size">{{ size(file.size) }}</span>
      <span class="file-actions">
        <button type="button" class="btn small" @click="open(false)"><Icon name="external" :size="14" />Openen</button>
        <button type="button" class="btn small" @click="open(true)">{{ revealName }}</button>
      </span>
    </div>
    <p v-if="changed" class="note warn" role="note">Het bestand is gewijzigd sinds het voorstel. Na Goedkeuren gaat de versie van toen naar e-Boekhouden.</p>
    <p v-if="error" class="note bad" role="alert">{{ error }}</p>
    <img v-if="url && shown() === 'image'" class="file-view image" :src="url" :alt="`Voorbeeld van ${file.name}`" />
    <iframe v-else-if="url && shown() === 'pdf'" class="file-view pdf" :src="url" :title="`Voorbeeld van ${file.name}`" />
    <p v-else-if="loading" class="file-hint">Voorbeeld laden…</p>
    <p v-else-if="!shown()" class="file-hint">Van dit soort bestand is geen voorbeeld; open het om het te bekijken.</p>
  </div>
</template>

<style scoped>
.file-preview { display: flex; flex-direction: column; gap: 8px; }
.file-head { display: flex; align-items: center; gap: 8px; min-width: 0; }
.file-name { flex: 1; min-width: 0; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.file-size { font-size: 11px; color: var(--muted); }
.file-actions { display: flex; gap: 6px; }
.file-view { width: 100%; border: 0.5px solid var(--line); border-radius: 10px; background: var(--sunken); }
.file-view.image { max-height: 420px; object-fit: contain; }
.file-view.pdf { height: 480px; }
.file-hint { margin: 0; font-size: 12px; color: var(--muted); }
.note { margin: 0; }
</style>
