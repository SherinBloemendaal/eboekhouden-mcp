<script setup lang="ts">
// The 1Password item picker inside the account sheet: asks `op` for the Login items once, then filters them
// as you type. e-Boekhouden items come first. Arrows and Enter pick, Esc closes the list (not the sheet).
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { filterOpItems, openOpInstall, opItems, type OpItem, type OpListError } from '../lib/accounts'
import Icon from './Icon.vue'

const props = defineProps<{ current?: string | null }>()
const emit = defineEmits<{ pick: [OpItem]; close: [] }>()

const items = ref<OpItem[]>([])
const loading = ref(true)
const error = ref<OpListError | null>(null)
const query = ref('')
const active = ref(0)
const search = ref<HTMLInputElement | null>(null)
const list = ref<HTMLElement | null>(null)

const groups = computed(() => filterOpItems(items.value, query.value))
/** The rows in screen order, for the arrow keys. */
const rows = computed(() => [...groups.value.suggested, ...groups.value.other])

async function load() {
  loading.value = true
  error.value = null
  try {
    items.value = await opItems()
    const current = rows.value.findIndex((i) => i.reference === props.current)
    active.value = current >= 0 ? current : 0
    await nextTick()
    search.value?.focus()
  } catch (e) {
    error.value = typeof e === 'object' && e && 'kind' in e ? (e as OpListError) : { kind: 'other', message: String(e) }
  } finally {
    loading.value = false
  }
}
onMounted(load)
watch(query, () => (active.value = 0))

function rowId(index: number) {
  return `op-item-${index}`
}

async function move(delta: number) {
  if (!rows.value.length) return
  active.value = (active.value + delta + rows.value.length) % rows.value.length
  await nextTick()
  list.value?.querySelector(`#${rowId(active.value)}`)?.scrollIntoView?.({ block: 'nearest' })
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'ArrowDown') {
    event.preventDefault()
    void move(1)
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    void move(-1)
  } else if (event.key === 'Enter') {
    event.preventDefault()
    const item = rows.value[active.value]
    if (item) emit('pick', item)
  } else if (event.key === 'Escape') {
    // Closes the list only; the sheet stays.
    event.stopPropagation()
    emit('close')
  }
}

const errorText = computed(() => {
  const e = error.value
  if (!e) return ''
  if (e.kind === 'missing') return 'De 1Password-CLI (op) is niet gevonden. Installeer hem en zet in 1Password bij Instellingen, Ontwikkelaar de koppeling met de CLI aan.'
  if (e.kind === 'locked') return '1Password is vergrendeld of gaf geen toestemming. Ontgrendel 1Password en probeer opnieuw.'
  return e.message
})
</script>

<template>
  <div class="picker">
    <Transition name="swap-state" mode="out-in">
      <p v-if="loading" key="loading" class="state" role="status"><span class="spinner" />1Password vraagt om toestemming…</p>
      <div v-else-if="error" key="error" class="notice" role="alert">
        <span class="notice-text"><Icon name="alert" :size="16" />{{ errorText }}</span>
        <span class="notice-actions">
          <button v-if="error.kind === 'missing'" type="button" class="btn small" @click="openOpInstall"><Icon name="external" :size="14" />Installatie-uitleg</button>
          <button type="button" class="btn small" @click="load"><Icon name="refresh" :size="14" />{{ error.kind === 'missing' ? 'Opnieuw zoeken' : 'Opnieuw proberen' }}</button>
        </span>
      </div>
      <div v-else key="list" class="box">
        <span class="search">
          <Icon name="search" :size="15" class="search-icon" />
          <input
            ref="search" v-model="query" class="field" type="search" placeholder="Zoek in 1Password" aria-label="Zoek in 1Password"
            role="combobox" aria-expanded="true" aria-controls="op-items" :aria-activedescendant="rows.length ? rowId(active) : undefined"
            spellcheck="false" autocomplete="off" @keydown="onKey"
          />
        </span>
        <div id="op-items" ref="list" class="list" role="listbox" aria-label="Logins in 1Password">
          <template v-for="(group, g) in [{ label: 'Voor e-Boekhouden', items: groups.suggested, offset: 0 }, { label: groups.suggested.length ? 'Andere logins' : 'Logins', items: groups.other, offset: groups.suggested.length }]" :key="g">
            <template v-if="group.items.length">
              <span class="group-label" role="presentation">{{ group.label }}</span>
              <button
                v-for="(item, i) in group.items" :id="rowId(group.offset + i)" :key="item.reference" type="button" role="option"
                class="row" :class="{ active: active === group.offset + i }" :aria-selected="item.reference === props.current"
                tabindex="-1" @mousemove="active = group.offset + i" @click="emit('pick', item)"
              >
                <span class="row-icon"><Icon name="key" :size="14" /></span>
                <span class="row-text"><span class="row-title">{{ item.title || 'Zonder titel' }}</span><span class="row-sub">{{ item.username || 'Geen gebruikersnaam' }}</span></span>
                <span v-if="item.vault" class="vault">{{ item.vault }}</span>
                <span class="check" :class="{ off: item.reference !== props.current }"><Icon name="check" :size="15" /></span>
              </button>
            </template>
          </template>
          <p v-if="!rows.length" class="empty">{{ items.length ? `Niets gevonden voor "${query}"` : 'Geen logins in 1Password' }}</p>
        </div>
        <span class="foot">{{ rows.length }} van {{ items.length }} logins · ↑↓ en Enter om te kiezen</span>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.picker { display: flex; flex-direction: column; min-width: 0; }
.state { margin: 0; display: flex; align-items: center; gap: 8px; padding: 12px; border-radius: 12px; background: var(--sunken); font-size: 12px; color: var(--muted); }
.spinner { box-sizing: border-box; width: 12px; height: 12px; border-radius: 50%; border: 1.5px solid var(--line); border-top-color: var(--label); animation: spin 800ms linear infinite; }
.notice { display: flex; flex-direction: column; gap: 10px; padding: 12px; border-radius: 12px; background: var(--warn-soft); color: var(--warn); font-size: 12px; line-height: 17px; }
.notice-text { display: flex; gap: 8px; align-items: flex-start; }
.notice-text :deep(svg) { flex: none; margin-top: 1px; }
.notice-actions { display: flex; gap: 8px; }
.box { display: flex; flex-direction: column; gap: 6px; padding: 8px; border-radius: 12px; background: var(--sunken); box-shadow: inset 0 0 0 0.5px var(--line); }
.search { position: relative; display: block; }
.search-icon { position: absolute; left: 9px; top: 6.5px; color: var(--muted); pointer-events: none; }
.field {
  box-sizing: border-box; width: 100%; height: 28px; padding: 0 10px 0 30px; border: 0; border-radius: 8px; background: var(--field-bg);
  box-shadow: inset 0 0 0 0.5px var(--field); outline: none; transition: box-shadow 150ms ease; -webkit-user-select: text; user-select: text;
}
.field:focus { box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.field::-webkit-search-cancel-button { display: none; }
.list { display: flex; flex-direction: column; gap: 1px; max-height: 236px; overflow-y: auto; }
.group-label { padding: 6px 8px 2px; font-size: 11px; font-weight: 600; color: var(--muted); }
.row {
  box-sizing: border-box; width: 100%; display: flex; align-items: center; gap: 10px; padding: 5px 8px; flex: none;
  border: 0; border-radius: 8px; background: transparent; text-align: left; transition: background-color 150ms ease;
}
.row.active { background: var(--hover); }
.row[aria-selected='true'] { background: var(--info-soft); }
.row-icon { width: 26px; height: 26px; flex: none; border-radius: 7px; background: var(--subtle); color: var(--label); display: flex; align-items: center; justify-content: center; }
.row-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
.row-title, .row-sub { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.row-title { font-size: 13px; font-weight: 500; }
.row-sub { font-size: 11px; color: var(--muted); }
.vault { flex: none; height: 18px; padding: 0 7px; border-radius: 999px; background: var(--subtle); color: var(--label); font-size: 11px; display: inline-flex; align-items: center; }
.check { color: var(--accent); display: flex; transition: opacity 150ms ease; }
.check.off { opacity: 0; }
.empty { margin: 0; padding: 14px 8px; font-size: 12px; color: var(--muted); text-align: center; }
.foot { padding: 0 4px 2px; font-size: 11px; color: var(--muted); }
.swap-state-enter-active { transition: opacity 180ms var(--ease); }
.swap-state-leave-active { transition: opacity 120ms cubic-bezier(0.4, 0, 1, 1); }
.swap-state-enter-from, .swap-state-leave-to { opacity: 0; }
@media (prefers-reduced-motion: reduce) { .spinner { animation-duration: 2400ms; } }
</style>
