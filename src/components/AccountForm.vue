<script setup lang="ts">
// Add or edit an account. Passwords go straight to the system store; the app keeps none.
import { computed, reactive, ref } from 'vue'
import { addAccount, updateAccount, type AccountView, type StoreKind } from '../lib/accounts'
import Sheet from './Sheet.vue'

const props = defineProps<{ account?: AccountView }>()
const emit = defineEmits<{ close: []; saved: [AccountView, boolean] }>()

const editing = computed(() => !!props.account)
const form = reactive({
  label: props.account?.label ?? '',
  store: (props.account?.store ?? 'onePassword') as StoreKind,
  opRef: props.account?.opRef ?? '',
  username: props.account?.store === 'system' ? props.account.username : '',
  password: '',
  isDefault: props.account?.isDefault ?? false,
})
const error = ref<string | null>(null)
const busy = ref(false)
const showPassword = ref(false)

async function save() {
  if (busy.value) return
  busy.value = true
  error.value = null
  const input = {
    label: form.label,
    username: form.store === 'system' ? form.username : (props.account?.username ?? ''),
    store: form.store,
    opRef: form.store === 'onePassword' ? form.opRef : null,
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
      <template v-if="form.store === 'onePassword'">
        <label class="field-block">
          <span class="label">1Password-item</span>
          <input v-model="form.opRef" class="field mono" placeholder="op://Werk/e-Boekhouden Studio Noord" spellcheck="false" />
          <span class="hint">EB MCP leest de velden username en password van dit item via de 1Password-CLI, alleen op het moment van inloggen.</span>
        </label>
      </template>
      <template v-else>
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
      </template>
      <label class="check">
        <input v-model="form.isDefault" type="checkbox" />
        <span>Standaardaccount voor AI-clients</span>
      </label>
      <p v-if="!editing" class="hint">Bij Opslaan logt EB MCP op de achtergrond één keer in, om de gegevens te controleren en de administratienaam op te halen.</p>
      <p v-if="error" class="note bad rise" role="alert">{{ error }}</p>
    </form>
    <template #footer>
      <button type="button" class="btn" @click="emit('close')">Annuleren</button>
      <button type="submit" form="account-form" class="btn primary" :disabled="busy">{{ busy ? 'Bezig…' : 'Opslaan' }}</button>
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
.ring { width: 16px; height: 16px; flex: none; box-sizing: border-box; border-radius: 50%; border: 1.5px solid var(--field); background: var(--field-bg); transition: border 150ms ease; margin-top: 1px; }
.option.on .ring { border: 5px solid var(--accent); }
.with-button { display: flex; gap: 6px; align-items: center; }
.check { display: flex; align-items: center; gap: 8px; }
.check input { accent-color: var(--accent); width: 14px; height: 14px; margin: 0; }
.note { padding: 9px 11px; border-radius: 10px; font-size: 12px; line-height: 16px; }
.note.warn { background: var(--warn-soft); color: var(--warn); }
.note.bad { background: var(--danger-soft); color: var(--danger); }
</style>
