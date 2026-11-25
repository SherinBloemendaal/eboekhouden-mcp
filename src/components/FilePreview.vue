<script setup lang="ts">
// The file a waiting proposal sends along (archief_uploaden), shown on Akkoord before Goedkeuren: an image as is,
// a PDF in the webview's own viewer, anything else by name. What is shown are the bytes the tool read when the
// proposal came up, which are the bytes that go out: not the file as it is on disk now.
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import Icon from './Icon.vue'
import { approvalFile, openApprovalFile, previewKind, type Attachment } from '../lib/approvals'
import { size } from '../lib/log'
import { inApp, revealName } from '../lib/server'

const props = defineProps<{ id: number; file: Attachment }>()

const url = ref<string | null>(null)
const changed = ref(false)
const error = ref<string | null>(null)
const loading = ref(false)
/** The webview could not draw the image (HEIC outside macOS). */
const broken = ref(false)
/** Which of the two buttons waits for the system. */
const opening = ref<'open' | 'reveal' | null>(null)
const kind = computed(() => previewKind(props.file.kind))
/** What stands in the place of the preview while there is none. */
const hint = computed(() => {
  if (!kind.value) return `Van dit soort bestand is geen voorbeeld; ${props.file.openable ? 'open het om het te bekijken' : 'bekijk het in zijn map'}.`
  if (broken.value) return `Dit voorbeeld kan hier niet worden getoond; ${props.file.openable ? 'open het bestand om het te bekijken' : 'bekijk het bestand in zijn map'}.`
  if (loading.value) return 'Voorbeeld laden…'
  return null
})

function clear() {
  if (url.value?.startsWith('blob:')) URL.revokeObjectURL(url.value)
  url.value = null
}

async function load() {
  clear()
  changed.value = false
  error.value = null
  broken.value = false
  if (!kind.value || !inApp()) return
  const id = props.id
  loading.value = true
  try {
    const preview = await approvalFile(id)
    if (id !== props.id) return
    changed.value = preview.changed
    const type = preview.kind ?? 'application/octet-stream'
    if (kind.value === 'image') {
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
  if (opening.value) return
  error.value = null
  opening.value = reveal ? 'reveal' : 'open'
  try {
    await openApprovalFile(props.id, reveal)
  } catch (e) {
    error.value = String(e)
  } finally {
    opening.value = null
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
        <!-- Only a PDF or an image is handed to its own app: opening anything else could run it. -->
        <button v-if="file.openable" type="button" class="btn small" :disabled="!!opening" :aria-busy="opening === 'open'" @click="open(false)"><span class="btn-label"><Icon name="external" :size="14" />Openen</span></button>
        <button type="button" class="btn small" :disabled="!!opening" :aria-busy="opening === 'reveal'" @click="open(true)"><span class="btn-label">{{ revealName }}</span></button>
      </span>
    </div>
    <Transition name="accordion">
      <div v-if="changed" class="accordion"><div><p class="note warn" role="note">Het bestand op deze computer is gewijzigd of weg sinds het voorstel. Je ziet hier de versie van toen: die gaat na Goedkeuren naar e-Boekhouden.</p></div></div>
    </Transition>
    <Transition name="accordion">
      <div v-if="error" class="accordion"><div><p class="note bad" role="alert">{{ error }}</p></div></div>
    </Transition>
    <!-- The preview has a fixed place, so nothing shifts when it arrives: the hint fades into the image or the PDF. -->
    <div v-if="kind || hint" class="file-stage" :class="{ tall: !!kind }">
      <Transition name="fade" mode="out-in">
        <img v-if="url && kind === 'image' && !broken" key="image" class="file-view image" :src="url" :alt="`Voorbeeld van ${file.name}`" @error="broken = true" />
        <iframe v-else-if="url && kind === 'pdf'" key="pdf" class="file-view pdf" :src="url" :title="`Voorbeeld van ${file.name}`" />
        <p v-else-if="hint" :key="hint" class="file-hint">{{ hint }}</p>
      </Transition>
    </div>
  </div>
</template>

<style scoped>
.file-preview { display: flex; flex-direction: column; gap: 8px; }
.file-head { display: flex; align-items: center; gap: 8px; min-width: 0; }
.file-name { flex: 1; min-width: 0; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.file-size { font-size: 11px; color: var(--muted); }
.file-actions { display: flex; gap: 6px; }
.file-stage { display: flex; align-items: center; justify-content: center; min-height: 18px; }
.file-stage.tall { height: 420px; border: 0.5px solid var(--line); border-radius: 10px; background: var(--sunken); overflow: hidden; }
.file-view { width: 100%; height: 100%; border: 0; }
.file-view.image { object-fit: contain; }
.file-hint { margin: 0; font-size: 12px; color: var(--muted); }
.file-stage:not(.tall) { justify-content: flex-start; }
.note { margin: 0; }
</style>
