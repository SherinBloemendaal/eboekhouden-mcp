<script setup lang="ts">
// "Terug naar Claude": the field where the user writes what the AI client should do differently, instead of
// approving or refusing. Mount it inside <Transition name="accordion"> so it opens and closes with its height.
import { nextTick, onMounted, ref } from 'vue'
import Icon from './Icon.vue'

const props = defineProps<{ client: string; busy: boolean; statement?: boolean }>()
const emit = defineEmits<{ send: [remark: string]; cancel: [] }>()

const remark = ref('')
const field = ref<HTMLTextAreaElement | null>(null)
onMounted(() => void nextTick(() => field.value?.focus()))

function send() {
  if (props.busy || !remark.value.trim()) return
  emit('send', remark.value)
}
</script>

<template>
  <div class="accordion return-box">
    <div>
      <div class="inner">
        <div class="head">
          <label class="title" for="return-remark">Wat moet {{ props.client }} anders doen?</label>
          <span class="chip">Er wordt niets gewijzigd</span>
        </div>
        <textarea
          id="return-remark" ref="field" v-model="remark" class="field" rows="3" maxlength="2000" :disabled="props.busy"
          placeholder="Bijvoorbeeld: deze regels horen op een andere rekening" @keydown.meta.enter.prevent="send" @keydown.ctrl.enter.prevent="send" @keydown.esc.stop="emit('cancel')"
        />
        <p class="hint">
          {{ props.client }} krijgt je tekst als antwoord op zijn aanroep<template v-if="props.statement">, samen met wat je in dit scherm hebt aangepast: de regels, de vinkjes en de kolommen</template>.
          Dit voorstel gaat uit de wachtrij en staat in Geschiedenis als Teruggestuurd; {{ props.client }} kan een nieuw voorstel doen.
        </p>
        <div class="buttons">
          <button type="button" class="btn" :disabled="props.busy" @click="emit('cancel')">Annuleren</button>
          <button type="button" class="btn primary send" :disabled="props.busy || !remark.trim()" :aria-busy="props.busy" @click="send">
            <span class="btn-label"><Icon name="reply" :size="14" />Stuur naar {{ props.client }}</span>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.inner { display: flex; flex-direction: column; gap: 8px; padding: 14px 0 2px; }
.head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.title { font-size: 13px; font-weight: 600; }
.chip { display: inline-flex; align-items: center; height: 18px; padding: 0 7px; border-radius: 999px; background: var(--subtle); color: var(--muted); font-size: 11px; font-weight: 500; white-space: nowrap; }
.field {
  box-sizing: border-box; width: 100%; min-height: 64px; padding: 8px 10px; border: 0; border-radius: 9px; resize: vertical;
  background: var(--field-bg); box-shadow: inset 0 0 0 0.5px var(--field); font: inherit; line-height: 18px; color: inherit; outline: none;
  transition: box-shadow 150ms ease;
}
.field:focus { box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.hint { margin: 0; font-size: 11px; line-height: 15px; color: var(--muted); }
.buttons { display: flex; justify-content: flex-end; gap: 8px; }
</style>
