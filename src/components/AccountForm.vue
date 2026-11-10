<script setup lang="ts">
// Add or edit an account. Passwords go straight to the system store; the app keeps none.
import { computed, reactive, ref } from 'vue'
import { addAccount, updateAccount, type AccountView, type OpItem, type StoreKind } from '../lib/accounts'
import Icon from './Icon.vue'
import OpItemPicker from './OpItemPicker.vue'
import Sheet from './Sheet.vue'

const props = defineProps<{ account?: AccountView }>()
const emit = defineEmits<{ close: []; saved: [AccountView, boolean] }>()

const editing = computed(() => !!props.account)
const form = reactive({
  label: props.account?.label ?? '',
  store: (props.account?.store ?? 'onePassword') as StoreKind,
  opRef: props.account?.opRef ?? '',
  opTitle: props.account?.opTitle ?? null as string | null,
  opVault: props.account?.opVault ?? null as string | null,
  username: props.account?.store === 'system' ? props.account.username : '',
  password: '',
  isDefault: props.account?.isDefault ?? false,
})
const error = ref<string | null>(null)
const busy = ref(false)
const showPassword = ref(false)

/** 1Password: the chosen item as a card, the list to pick from, or a reference typed by hand. */
type OpMode = 'card' | 'picker' | 'manual'
const opMode = ref<OpMode>(props.account?.opRef ? 'card' : 'picker')
/** The list asks 1Password for consent, so it only loads after a click. */
const pickerOpen = ref(false)

/** The username shown on the card: from the picked item, else the one read at the last sign-in. */
const pickedUser = ref<string | null>(props.account?.store === 'onePassword' ? props.account.username || null : null)
const cardSub = computed(() =>
  form.opTitle ? [form.opVault, pickedUser.value].filter(Boolean).join(' · ') || '1Password' : 'Zelf ingevulde verwijzing',
)

function pick(item: OpItem) {
  pickedUser.value = item.username || null
  form.opRef = item.reference
  form.opTitle = item.title || null
  form.opVault = item.vault || null
  if (!form.label.trim() && item.title) form.label = item.title.slice(0, 60)
  opMode.value = 'card'
  pickerOpen.value = false
}

function changeItem() {
  opMode.value = 'picker'
  pickerOpen.value = true
}

function closePicker() {
  pickerOpen.value = false
  if (form.opRef) opMode.value = 'card'
}

function typeByHand() {
  opMode.value = 'manual'
  pickerOpen.value = false
}

/** A reference typed by hand has no title or vault to show. */
function onManualInput() {
  form.opTitle = null
  form.opVault = null
}

async function save() {
  if (busy.value) return
  busy.value = true
  error.value = null
  const input = {
    label: form.label,
    username: form.store === 'system' ? form.username : (props.account?.username ?? ''),
    store: form.store,
    opRef: form.store === 'onePassword' ? form.opRef : null,
    opTitle: form.store === 'onePassword' ? form.opTitle : null,
    opVault: form.store === 'onePassword' ? form.opVault : null,
    isDefault: form.isDefault,
  }
  try {
    const password = form.store === 'system' && form.password ? form.password : null
    const saved = props.account ? await updateAccount(props.account.id, input, password) : await addAccount(input, password)
    form.password = ''
    emit('saved', saved, !props.account)
  } catch (e) {
    error.value = String(e)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <Sheet :title="editing ? `${account!.label} bewerken` : 'Account toevoegen'" :width="560" @close="emit('close')">
    <form id="account-form" class="form" @submit.prevent="save">
      <p v-if="editing && account!.session && !account!.session.expired" class="note warn">
        {{ account!.label }} is ingelogd. Wijzigingen gelden vanaf de volgende keer inloggen; de lopende sessie blijft.
      </p>
      <label class="field-block">
        <span class="label">Naam in EB MCP</span>
        <input v-model="form.label" class="field" maxlength="60" placeholder="Studio Noord" required />
      </label>
      <div v-if="editing" class="field-block">
        <span class="label">Administratie</span>
        <input class="field" readonly :value="account!.administration ?? 'Na de eerste keer inloggen'" />
      </div>
      <div class="field-block" role="radiogroup" aria-label="Wachtwoord bewaren in">
        <span class="label">Wachtwoord bewaren in</span>
        <div class="options">
          <button type="button" role="radio" :aria-checked="form.store === 'onePassword'" class="option" :class="{ on: form.store === 'onePassword' }" @click="form.store = 'onePassword'">
            <span class="ring" />
            <span><strong>1Password</strong><small>EB MCP bewaart alleen een op://-verwijzing. 1Password vraagt zelf om toestemming bij het inloggen.</small></span>
          </button>
          <button type="button" role="radio" :aria-checked="form.store === 'system'" class="option" :class="{ on: form.store === 'system' }" @click="form.store = 'system'">
            <span class="ring" />
            <span><strong>Sleutelhanger</strong><small>Gebruikersnaam en wachtwoord in je sleutelhanger, met Touch ID bij elk gebruik.</small></span>
          </button>
        </div>
      </div>
      <Transition name="accordion" mode="out-in">
        <div v-if="form.store === 'onePassword'" key="onePassword" class="accordion fields">
          <div class="fields-inner">
            <div class="field-block">
              <span id="op-item-label" class="label">1Password-item</span>
              <Transition name="swap-op" mode="out-in">
                <div v-if="opMode === 'card'" key="card" class="op-card" role="group" aria-labelledby="op-item-label">
                  <span class="op-icon"><Icon name="key" :size="16" /></span>
                  <span class="op-text">
                    <span class="op-title" :class="{ mono: !form.opTitle }">{{ form.opTitle ?? form.opRef }}</span>
                    <span class="op-sub">{{ cardSub }}</span>
                  </span>
                  <button type="button" class="btn small" @click="changeItem">Wijzigen</button>
                </div>
                <div v-else-if="opMode === 'picker'" key="picker" class="op-picker">
                  <Transition name="swap-op" mode="out-in">
                    <OpItemPicker v-if="pickerOpen" :current="form.opRef || null" @pick="pick" @close="closePicker" />
                    <button v-else type="button" class="btn op-choose" @click="pickerOpen = true"><Icon name="search" :size="15" />Kies uit 1Password…</button>
                  </Transition>
                </div>
                <input
                  v-else key="manual" v-model="form.opRef" class="field mono" placeholder="op://Werk/e-Boekhouden Studio Noord"
                  aria-labelledby="op-item-label" spellcheck="false" @input="onManualInput"
                />
              </Transition>
              <span class="op-links">
                <button v-if="opMode !== 'manual'" type="button" class="link" @click="typeByHand">Verwijzing zelf invullen</button>
                <button v-else type="button" class="link" @click="changeItem">Kies uit de lijst</button>
              </span>
              <span class="hint">EB MCP leest gebruikersnaam en wachtwoord van dit item via de 1Password-CLI, alleen op het moment van inloggen. Het bewaart alleen de verwijzing.</span>
            </div>
          </div>
        </div>
        <div v-else key="system" class="accordion fields">
          <div class="fields-inner">
            <label class="field-block">
              <span class="label">Gebruikersnaam of e-mailadres</span>
              <input v-model="form.username" class="field" autocomplete="off" spellcheck="false" />
            </label>
            <label class="field-block">
              <span class="label">Wachtwoord</span>
              <span class="with-button">
                <input v-model="form.password" class="field" :type="showPassword ? 'text' : 'password'" autocomplete="new-password" :placeholder="editing && account!.store === 'system' ? 'Leeg laten om het huidige te houden' : ''" />
                <button type="button" class="btn small ghost" :aria-pressed="showPassword" @click="showPassword = !showPassword">{{ showPassword ? 'Verbergen' : 'Tonen' }}</button>
              </span>
              <span class="hint">Gaat direct de sleutelhanger in. EB MCP schrijft het nergens anders weg.</span>
            </label>
          </div>
        </div>
      </Transition>
      <label class="check">
        <input v-model="form.isDefault" type="checkbox" />
        <span>Standaardaccount voor AI-clients</span>
      </label>
      <p v-if="!editing" class="hint">Bij Opslaan logt EB MCP op de achtergrond één keer in, om de gegevens te controleren en de administratienaam op te halen.</p>
      <p v-if="error" class="note bad rise" role="alert">{{ error }}</p>
    </form>
    <template #footer>
      <button type="button" class="btn" @click="emit('close')">Annuleren</button>
      <button type="submit" form="account-form" class="btn primary" :disabled="busy" :aria-busy="busy"><span class="btn-label">Opslaan</span></button>
    </template>
  </Sheet>
</template>

<style scoped>
.form { display: flex; flex-direction: column; gap: 14px; }
.field-block { display: flex; flex-direction: column; gap: 6px; }
.label { font-size: 12px; font-weight: 600; color: var(--label); }
.field {
  box-sizing: border-box; width: 100%; height: 28px; padding: 0 10px; border: 0; border-radius: 8px; background: var(--field-bg);
  box-shadow: inset 0 0 0 0.5px var(--field); outline: none; transition: box-shadow 150ms ease; -webkit-user-select: text; user-select: text;
}
.field:focus { box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.field[readonly] { background: var(--sunken); color: var(--label); }
.hint { font-size: 11px; line-height: 15px; color: var(--muted); }
.options { display: flex; gap: 10px; }
.option {
  flex: 1; display: flex; gap: 10px; align-items: flex-start; padding: 12px; border: 0; border-radius: 12px; text-align: left;
  background: var(--group); box-shadow: inset 0 0 0 0.5px var(--line); transition: box-shadow 150ms ease, background-color 150ms ease;
}
.option:hover { box-shadow: inset 0 0 0 0.5px var(--field); }
.option.on { background: var(--info-soft); box-shadow: inset 0 0 0 1.5px var(--accent); }
.option span { display: flex; flex-direction: column; gap: 4px; }
.option small { font-size: 11px; line-height: 15px; color: var(--muted); }
.ring { position: relative; width: 16px; height: 16px; flex: none; box-sizing: border-box; border-radius: 50%; border: 1.5px solid var(--field); background: var(--field-bg); margin-top: 1px; }
.ring::before, .ring::after { content: ''; position: absolute; border-radius: 50%; opacity: 0; transition: opacity 150ms ease, transform 150ms var(--ease); }
.ring::before { inset: -1.5px; background: var(--accent); }
.ring::after { inset: 3.5px; background: #fff; transform: scale(0.4); }
.option.on .ring::before { opacity: 1; }
.option.on .ring::after { opacity: 1; transform: none; }
.fields-inner { display: flex; flex-direction: column; gap: 14px; }
.fields.accordion-leave-active, .fields.accordion-leave-active > * { transition-duration: 120ms; }
.op-card { display: flex; align-items: center; gap: 12px; padding: 10px 12px; border-radius: 12px; background: var(--sunken); box-shadow: inset 0 0 0 0.5px var(--line); }
.op-icon { width: 32px; height: 32px; flex: none; border-radius: 8px; background: var(--info-soft); color: var(--info); display: flex; align-items: center; justify-content: center; }
.op-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.op-title, .op-sub { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.op-title { font-size: 13px; font-weight: 600; }
.op-sub { font-size: 12px; color: var(--muted); }
.op-choose { align-self: flex-start; }
.op-picker { display: flex; flex-direction: column; }
.op-links { display: flex; }
.link { padding: 0; border: 0; background: transparent; color: var(--accent); font-size: 12px; transition: opacity 150ms ease; }
.link:hover { opacity: 0.75; }
.swap-op-enter-active { transition: opacity 180ms var(--ease), transform 180ms var(--ease); }
.swap-op-leave-active { transition: opacity 120ms cubic-bezier(0.4, 0, 1, 1); }
.swap-op-enter-from { opacity: 0; transform: translateY(-4px); }
.swap-op-leave-to { opacity: 0; }
@media (prefers-reduced-motion: reduce) { .swap-op-enter-from { transform: none; } }
.with-button { display: flex; gap: 6px; align-items: center; }
.check { display: flex; align-items: center; gap: 8px; }
.check input { accent-color: var(--accent); width: 14px; height: 14px; margin: 0; }
.note { padding: 9px 11px; border-radius: 10px; font-size: 12px; line-height: 16px; }
.note.warn { background: var(--warn-soft); color: var(--warn); }
.note.bad { background: var(--danger-soft); color: var(--danger); }
@media (prefers-reduced-motion: reduce) { .ring::after { transform: none; } }
</style>
