<script setup lang="ts">
// "Afschrift controleren": the lines of the statement a waiting proposal reads in, checked by the user before
// anything goes to e-Boekhouden. On the left what e-Boekhouden's own upload screen asks (the account, the columns,
// the start line, ontdubbelen) and the rules that leave lines out; on the right every line with a tick. Only the
// ticked lines go out. Every change goes to the app at once (statement.rs), so the proposal in the queue says the
// same as this sheet. Mount it inside <Transition name="modal">.
import { computed, onMounted, reactive, ref, watch } from 'vue'
import Icon from './Icon.vue'
import ReturnBox from './ReturnBox.vue'
import { useDialogLayer } from './Sheet.vue'
import {
  approvalStatement, dutchDate, editStatement, euroCents, filterRows, outReason, testHits,
  type Proposal, type StatementEdit, type StatementMapping, type StatementRow, type StatementTest, type StatementView, type Ymd,
} from '../lib/approvals'

const props = defineProps<{ proposal: Proposal; busy: 'approve' | 'refuse' | 'return' | null }>()
const emit = defineEmits<{ close: []; decide: [approve: boolean]; giveBack: [remark: string] }>()

const panel = ref<HTMLElement | null>(null)
useDialogLayer(panel, () => (adding.value ? (adding.value = false) : returning.value ? (returning.value = false) : emit('close')), (el) => el.querySelector<HTMLElement>('.close'))

const view = ref<StatementView | null>(null)
const error = ref<string | null>(null)
/** The edit on its way to the app: the control that sent it shows that it waits, and no second edit starts. */
const saving = ref(false)
const tab = ref<'all' | 'in' | 'out'>('all')
const query = ref('')
/** How many lines are drawn; a long statement grows by a click instead of drawing thousands of lines at once. */
const PAGE = 300
const shown = ref(PAGE)
const adding = ref(false)
const returning = ref(false)

onMounted(async () => {
  try {
    view.value = await approvalStatement(props.proposal.id)
  } catch (e) {
    error.value = String(e)
  }
})

async function edit(change: StatementEdit): Promise<boolean> {
  if (saving.value) return false
  saving.value = true
  error.value = null
  try {
    view.value = await editStatement(props.proposal.id, change)
    return true
  } catch (e) {
    error.value = String(e)
    return false
  } finally {
    saving.value = false
  }
}

const rows = computed(() => (view.value ? filterRows(view.value.rows, tab.value, query.value) : []))
const visible = computed(() => rows.value.slice(0, shown.value))
watch([tab, query], () => (shown.value = PAGE))

const counts = computed(() => {
  const totals = view.value?.totals
  return totals ? { all: totals.lines + totals.unreadable, in: totals.included, out: totals.skipped + totals.unreadable } : { all: 0, in: 0, out: 0 }
})

/** The lines of the list that can be ticked at all. */
const tickable = computed(() => rows.value.filter((r) => r.out?.kind !== 'unreadable'))
const allState = computed<'true' | 'false' | 'mixed'>(() => {
  const on = tickable.value.filter((r) => r.included).length
  return on === 0 ? 'false' : on === tickable.value.length ? 'true' : 'mixed'
})

function toggleAll() {
  const included = allState.value !== 'true'
  void edit({ kind: 'tick', lines: tickable.value.map((r) => r.line), included })
}

function toggle(row: StatementRow) {
  if (row.out?.kind === 'unreadable') return
  void edit({ kind: 'tick', lines: [row.line], included: !row.included })
}

function setMapping(part: Partial<StatementMapping>) {
  if (!view.value) return
  void edit({ kind: 'mapping', mapping: { ...view.value.mapping, ...part } })
}

function setStart(event: Event) {
  const input = event.target as HTMLInputElement
  const start = Number(input.value)
  if (!view.value || !Number.isInteger(start) || start < 1) {
    input.value = String(view.value?.mapping.start ?? 1)
    return
  }
  if (start !== view.value.mapping.start) setMapping({ start })
}

// ---- A new rule ----

type RuleKind = StatementTest['kind']
const KINDS: { value: RuleKind; label: string }[] = [
  { value: 'contains', label: 'Omschrijving bevat' },
  { value: 'until', label: 'Datum tot en met' },
  { value: 'from', label: 'Datum vanaf' },
  { value: 'column', label: 'Kolom is' },
  { value: 'amount', label: 'Bedrag is' },
  { value: 'incoming', label: 'Bedrag bij' },
  { value: 'outgoing', label: 'Bedrag af' },
  { value: 'cancelling', label: 'Valt weg tegen een regel van dezelfde dag' },
]
const draft = reactive({ kind: 'contains' as RuleKind, text: '', date: '', column: 0, value: '', amount: '' })

function openAdd() {
  Object.assign(draft, { kind: 'contains', text: '', date: '', column: 0, value: '', amount: '' })
  returning.value = false
  adding.value = true
}

/** The rule being made, or null while it is not complete yet. */
const drafted = computed<StatementTest | null>(() => {
  switch (draft.kind) {
    case 'contains': return draft.text.trim() ? { kind: 'contains', text: draft.text.trim() } : null
    case 'until':
    case 'from': {
      const [y, m, d] = draft.date.split('-').map(Number)
      return y && m && d ? { kind: draft.kind, date: [y, m, d] as Ymd } : null
    }
    case 'column': return draft.value.trim() ? { kind: 'column', column: draft.column, value: draft.value.trim() } : null
    case 'amount': {
      const cents = Math.round(Number(draft.amount.replace(/\./g, '').replace(',', '.')) * 100)
      return draft.amount.trim() && Number.isFinite(cents) ? { kind: 'amount', cents } : null
    }
    default: return { kind: draft.kind }
  }
})

/** The lines the rule being made would leave out: they light up in the list while it is typed. */
const hits = computed(() => {
  const test = drafted.value
  if (!adding.value || !test || !view.value) return new Set<number>()
  return new Set(view.value.rows.filter((row) => row.included && testHits(test, row)).map((row) => row.line))
})
const hitSum = computed(() => (view.value?.rows ?? []).filter((r) => hits.value.has(r.line)).reduce((sum, r) => sum + (r.cents ?? 0), 0))

async function addRule() {
  const test = drafted.value
  if (!test) return
  if (await edit({ kind: 'addRule', test })) adding.value = false
}

const lines = (n: number) => (n === 1 ? '1 regel' : `${n} regels`)
const client = computed(() => props.proposal.client)

const period = computed(() => {
  const totals = view.value?.totals
  return totals?.first && totals.last ? `${dutchDate(totals.first)} t/m ${dutchDate(totals.last)}` : ''
})
</script>

<template>
  <div class="scrim" data-tauri-drag-region>
    <div ref="panel" class="sheet" role="dialog" aria-modal="true" tabindex="-1" aria-label="Afschrift controleren">
      <header class="head">
        <span class="titles">
          <h2>Afschrift controleren</h2>
          <span class="sub"><span class="mono">{{ view?.fileName ?? props.proposal.statement?.fileName }}</span> · {{ props.proposal.account }} · voorstel van {{ client }}</span>
        </span>
        <span class="pill">#{{ props.proposal.id }}</span>
        <button type="button" class="icon-btn close" aria-label="Sluiten" title="Sluiten" @click="emit('close')"><Icon name="x" :size="14" /></button>
      </header>

      <Transition name="fade" mode="out-in">
        <p v-if="!view && error" key="error" class="state note bad selectable" role="alert">{{ error }}</p>
        <p v-else-if="!view" key="loading" class="state loading" aria-busy="true">Afschrift laden…</p>
        <div v-else key="body" class="body">
          <aside class="side" aria-label="Instellingen van het inlezen">
            <div class="group">
              <label class="section-label" for="st-ledger">Rekening in e-Boekhouden</label>
              <select id="st-ledger" class="select" :value="view.ledger" :disabled="saving" @change="edit({ kind: 'ledger', index: Number(($event.target as HTMLSelectElement).value) })">
                <option v-for="(ledger, i) in view.ledgers" :key="ledger.id" :value="i">{{ ledger.code }} {{ ledger.name }}</option>
              </select>
            </div>
            <div class="group">
              <span class="section-label">Kolommen uit het bestand</span>
              <div class="map">
                <label for="st-date">Datum</label>
                <select id="st-date" class="select" :value="view.mapping.date" :disabled="saving" @change="setMapping({ date: Number(($event.target as HTMLSelectElement).value) })">
                  <option v-for="(name, i) in view.header" :key="i" :value="i">{{ name }}</option>
                </select>
                <label for="st-description">Omschrijving</label>
                <select id="st-description" class="select" :value="view.mapping.description" :disabled="saving" @change="setMapping({ description: Number(($event.target as HTMLSelectElement).value) })">
                  <option v-for="(name, i) in view.header" :key="i" :value="i">{{ name }}</option>
                </select>
                <label for="st-amount">Bedrag</label>
                <select id="st-amount" class="select" :value="view.mapping.amount" :disabled="saving" @change="setMapping({ amount: Number(($event.target as HTMLSelectElement).value) })">
                  <option v-for="(name, i) in view.header" :key="i" :value="i">{{ name }}</option>
                </select>
                <label for="st-start">Starten vanaf regel</label>
                <input id="st-start" class="select number" type="number" min="1" :value="view.mapping.start" :disabled="saving" @change="setStart" />
              </div>
            </div>
            <div class="toggle">
              <button type="button" role="switch" class="switch" :aria-checked="view.dedupe" aria-label="Ontdubbelen" :disabled="saving" @click="edit({ kind: 'dedupe', on: !view.dedupe })"><span /></button>
              <span class="toggle-text"><strong>Ontdubbelen</strong><small>e-Boekhouden slaat over wat er al staat</small></span>
            </div>

            <div class="group rules">
              <span class="section-label">Overslaan</span>
              <TransitionGroup name="accordion" tag="ul" class="rule-list">
                <li v-for="(rule, i) in view.rules" :key="rule.label" class="accordion">
                  <div>
                    <div class="rule">
                      <button type="button" role="switch" class="switch" :aria-checked="rule.on" :aria-label="rule.label" :disabled="saving" @click="edit({ kind: 'toggleRule', index: i, on: !rule.on })"><span /></button>
                      <span class="rule-text">
                        <strong>{{ rule.label }}</strong>
                        <small>{{ rule.on ? lines(rule.count) : 'Staat uit' }}<template v-if="rule.why"> · {{ rule.why }}</template></small>
                      </span>
                      <span class="chip" :class="rule.by === 'client' ? 'back' : 'run'">{{ rule.by === 'client' ? client : 'Jij' }}</span>
                      <button type="button" class="icon-btn remove" :aria-label="`Regel verwijderen: ${rule.label}`" title="Verwijderen" :disabled="saving" @click="edit({ kind: 'removeRule', index: i })"><Icon name="x" :size="12" /></button>
                    </div>
                  </div>
                </li>
              </TransitionGroup>
              <p v-if="!view.rules.length" class="none">Geen regels: alles wat is aangevinkt gaat mee.</p>
              <button type="button" class="btn small ghost add" :disabled="saving" @click="openAdd"><Icon name="plus" :size="13" />Regel toevoegen</button>
              <p class="foot-note">Een regel zet zijn regels uit; een vinkje in de lijst gaat daar altijd boven.<template v-if="view.empty"> {{ view.empty === 1 ? '1 lege regel' : `${view.empty} lege regels` }} in het bestand {{ view.empty === 1 ? 'telt' : 'tellen' }} niet mee.</template></p>
            </div>

            <Transition name="pop">
              <form v-if="adding" class="adder" role="dialog" aria-label="Regels overslaan" @submit.prevent="addRule">
                <strong>Regels overslaan</strong>
                <div class="map">
                  <label for="st-kind">Waar</label>
                  <select id="st-kind" v-model="draft.kind" class="select">
                    <option v-for="k in KINDS" :key="k.value" :value="k.value">{{ k.label }}</option>
                  </select>
                  <template v-if="draft.kind === 'contains'">
                    <label for="st-text">Tekst</label>
                    <input id="st-text" v-model="draft.text" class="select" type="text" autocomplete="off" />
                  </template>
                  <template v-else-if="draft.kind === 'until' || draft.kind === 'from'">
                    <label for="st-day">Datum</label>
                    <input id="st-day" v-model="draft.date" class="select" type="date" />
                  </template>
                  <template v-else-if="draft.kind === 'column'">
                    <label for="st-column">Kolom</label>
                    <select id="st-column" v-model.number="draft.column" class="select">
                      <option v-for="(name, i) in view.header" :key="i" :value="i">{{ name }}</option>
                    </select>
                    <label for="st-value">Waarde</label>
                    <input id="st-value" v-model="draft.value" class="select" type="text" autocomplete="off" />
                  </template>
                  <template v-else-if="draft.kind === 'amount'">
                    <label for="st-cents">Bedrag</label>
                    <input id="st-cents" v-model="draft.amount" class="select number" type="text" inputmode="decimal" placeholder="-24,00" autocomplete="off" />
                  </template>
                </div>
                <p class="adder-hits" role="status">
                  <template v-if="draft.kind === 'cancelling'">EB MCP zoekt de paren zelf: per dag een regel bij en een regel af met hetzelfde bedrag.</template>
                  <template v-else-if="!drafted">Vul in waar de regel op let.</template>
                  <template v-else>Raakt <strong>{{ lines(hits.size) }}</strong><template v-if="hits.size">, samen {{ euroCents(hitSum) }}. Ze lichten op in de lijst.</template><template v-else>.</template></template>
                </p>
                <div class="adder-buttons">
                  <button type="button" class="btn small" @click="adding = false">Annuleren</button>
                  <button type="submit" class="btn small primary" :disabled="!drafted || saving" :aria-busy="saving"><span class="btn-label">Voeg toe</span></button>
                </div>
              </form>
            </Transition>
          </aside>

          <div class="main">
            <div class="bar">
              <div class="tabs" role="tablist" aria-label="Welke regels">
                <button type="button" role="tab" class="tab" :class="{ on: tab === 'all' }" :aria-selected="tab === 'all'" @click="tab = 'all'">Alle<span class="count">{{ counts.all }}</span></button>
                <button type="button" role="tab" class="tab" :class="{ on: tab === 'in' }" :aria-selected="tab === 'in'" @click="tab = 'in'">Gaat mee<span class="count">{{ counts.in }}</span></button>
                <button type="button" role="tab" class="tab" :class="{ on: tab === 'out' }" :aria-selected="tab === 'out'" @click="tab = 'out'">Overgeslagen<span class="count">{{ counts.out }}</span></button>
              </div>
              <input v-model="query" class="search" type="search" placeholder="Zoek in de regels" aria-label="Zoek in de regels" />
            </div>
            <Transition name="accordion">
              <div v-if="error" class="accordion">
                <div><p class="note bad inline selectable" role="alert">{{ error }}</p></div>
              </div>
            </Transition>
            <div class="table" role="table" aria-label="Regels van het afschrift" :aria-busy="saving">
              <div class="tr th" role="row">
                <span role="columnheader">
                  <button type="button" role="checkbox" class="box" :class="{ on: allState !== 'false' }" :aria-checked="allState" aria-label="Alle regels in deze lijst" :disabled="saving || !tickable.length" @click="toggleAll">
                    <span v-if="allState === 'mixed'" class="dash" /><Icon v-else-if="allState === 'true'" name="check" :size="11" />
                  </button>
                </span>
                <span role="columnheader">Regel</span>
                <span role="columnheader">Datum</span>
                <span role="columnheader">Omschrijving</span>
                <span role="columnheader" class="num">Bedrag €</span>
              </div>
              <div class="rows">
                <div v-for="row in visible" :key="row.line" class="tr" role="row" :class="{ off: !row.included, hit: hits.has(row.line) }">
                  <span role="cell">
                    <button
                      type="button" role="checkbox" class="box" :class="{ on: row.included }" :aria-checked="row.included" :aria-label="`Regel ${row.line}: ${row.description}`"
                      :disabled="saving || row.out?.kind === 'unreadable'" @click="toggle(row)"
                    ><Icon v-if="row.included" name="check" :size="11" /></button>
                  </span>
                  <span role="cell" class="line mono">{{ row.line }}</span>
                  <span role="cell" class="date">{{ dutchDate(row.date) }}</span>
                  <span role="cell" class="desc">
                    <span class="desc-text" :title="row.cells.join(' · ')">{{ row.description || row.cells.join(' ') }}</span>
                    <span v-if="hits.has(row.line)" class="chip warn">Valt onder de nieuwe regel</span>
                    <span v-else-if="row.out" class="chip" :class="row.out.kind === 'unreadable' ? 'bad' : 'off'">{{ outReason(row.out, view.rules) }}</span>
                    <span v-else-if="row.overruled !== null" class="chip run">Zelf aangezet</span>
                  </span>
                  <span role="cell" class="num amount">{{ row.cents === null ? '' : euroCents(row.cents) }}</span>
                </div>
                <p v-if="!rows.length" class="none center">Geen regels voor deze keuze.</p>
                <button v-if="rows.length > shown" type="button" class="btn small more" @click="shown += PAGE">Toon nog {{ Math.min(PAGE, rows.length - shown) }} van {{ rows.length - shown }}</button>
              </div>
            </div>
          </div>
        </div>
      </Transition>

      <footer class="foot">
        <Transition name="accordion">
          <ReturnBox v-if="returning" :client="client" :busy="props.busy === 'return'" statement @send="emit('giveBack', $event)" @cancel="returning = false" />
        </Transition>
        <div class="foot-row">
          <span v-if="view" class="summary" role="status">
            <strong>{{ lines(view.totals.included) }} {{ view.totals.included === 1 ? 'gaat' : 'gaan' }} mee</strong>
            <small><template v-if="period">{{ period }} · </template>samen {{ euroCents(view.totals.cents) }} · {{ view.totals.skipped }} overgeslagen<template v-if="view.totals.unreadable"> · {{ view.totals.unreadable }} niet te lezen</template></small>
          </span>
          <span class="actions">
            <button type="button" class="btn refuse" :disabled="!!props.busy" :aria-busy="props.busy === 'refuse'" @click="emit('decide', false)"><span class="btn-label">Weigeren</span></button>
            <button type="button" class="btn give-back" :disabled="!!props.busy || returning" @click="(adding = false), (returning = true)"><Icon name="reply" :size="14" />Terug naar {{ client }}</button>
            <button type="button" class="btn primary approve" :disabled="!!props.busy || saving || !view || !view.totals.included" :aria-busy="props.busy === 'approve'" @click="emit('decide', true)">
              <span class="btn-label">Inlezen ({{ view?.totals.included ?? 0 }})</span>
            </button>
          </span>
        </div>
      </footer>
    </div>
  </div>
</template>

<style scoped>
.scrim {
  position: fixed; inset: 0; z-index: 20; display: flex; justify-content: center; align-items: flex-start;
  padding-top: 44px; background: rgba(0, 0, 0, 0.18); animation: fade 150ms ease both;
}
.scrim.modal-leave-active { animation: fade-out 140ms cubic-bezier(0.4, 0, 1, 1) both; pointer-events: none; }
.modal-leave-active .sheet { animation: lift 140ms cubic-bezier(0.4, 0, 1, 1) both; }
.sheet {
  width: 1260px; max-width: calc(100% - 32px); height: calc(100% - 44px - 20px); box-sizing: border-box; border-radius: 20px; overflow: hidden;
  display: flex; flex-direction: column; outline: none; container: sheet / inline-size;
  background: var(--glass-pop); -webkit-backdrop-filter: blur(40px) saturate(190%); backdrop-filter: blur(40px) saturate(190%);
  box-shadow: var(--pop-shadow); animation: drop 260ms var(--ease) both; transform-origin: 50% 0;
}
.head { flex: none; display: flex; align-items: center; gap: 10px; padding: 14px 16px 14px 20px; border-bottom: 0.5px solid var(--line-soft); }
.titles { min-width: 0; display: flex; flex-direction: column; gap: 1px; }
h2 { margin: 0; font-size: 15px; font-weight: 700; }
.sub { font-size: 11px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.pill { flex: none; height: 20px; padding: 0 7px; border-radius: 6px; background: var(--subtle); font-family: var(--mono); font-size: 12px; font-weight: 600; display: inline-flex; align-items: center; font-variant-numeric: tabular-nums; }
.close { margin-left: auto; }
.icon-btn { flex: none; width: 24px; height: 24px; padding: 0; border: 0; border-radius: 7px; background: transparent; color: var(--muted); display: inline-flex; align-items: center; justify-content: center; transition: background-color 150ms ease, color 150ms ease, transform 120ms ease; }
.icon-btn:hover { background: var(--hover); color: var(--text); }
.icon-btn:active { transform: scale(0.92); }
.icon-btn:focus-visible { outline: none; box-shadow: 0 0 0 3px rgba(10, 132, 255, 0.42); }
.state { flex: 1; margin: 20px; }
.loading { color: var(--muted); font-size: 12px; }

.body { flex: 1; min-height: 0; display: flex; }
.side { position: relative; width: 332px; flex: none; box-sizing: border-box; padding: 16px 20px; border-right: 0.5px solid var(--line-soft); display: flex; flex-direction: column; gap: 16px; overflow-y: auto; }
.group { display: flex; flex-direction: column; gap: 6px; }
.map { display: grid; grid-template-columns: 108px minmax(0, 1fr); align-items: center; gap: 6px 8px; }
.map label { font-size: 12px; color: var(--muted); }
.select {
  box-sizing: border-box; width: 100%; height: 26px; padding: 0 8px; border: 0; border-radius: 7px; background: var(--field-bg);
  box-shadow: inset 0 0 0 0.5px var(--field); font: inherit; font-size: 12px; color: inherit; outline: none; transition: box-shadow 150ms ease, opacity 150ms ease;
}
.select:hover:not(:disabled) { box-shadow: inset 0 0 0 0.5px var(--field), 0 0 0 3px var(--hover); }
.select:focus-visible { box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.select:disabled { opacity: 0.6; }
.number { font-variant-numeric: tabular-nums; }
.toggle, .rule { display: flex; align-items: center; gap: 8px; }
.toggle-text, .rule-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; font-size: 12px; }
.toggle-text strong, .rule-text strong { font-weight: 500; overflow-wrap: anywhere; }
.toggle-text small, .rule-text small { font-size: 11px; color: var(--muted); }
.switch { position: relative; width: 30px; height: 18px; flex: none; padding: 0; border: 0; border-radius: 999px; background: var(--subtle); box-shadow: inset 0 0 0 0.5px var(--line); transition: background-color 200ms var(--ease), opacity 150ms ease; }
.switch span { position: absolute; top: 2px; left: 2px; width: 14px; height: 14px; border-radius: 50%; background: #fff; box-shadow: 0 1px 3px rgba(0, 0, 0, 0.25); transition: transform 200ms var(--ease); }
.switch[aria-checked='true'] { background: var(--success-solid); }
.switch[aria-checked='true'] span { transform: translateX(12px); }
.switch:focus-visible { outline: none; box-shadow: 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.switch:disabled { opacity: 0.6; }
.rules { flex: 1; min-height: 0; }
.rule-list { margin: 0 -8px; padding: 0; list-style: none; }
.rule { padding: 7px 8px; border-radius: 9px; transition: background-color 150ms ease; }
.rule:hover { background: var(--hover); }
.remove { opacity: 0; }
.rule:hover .remove, .remove:focus-visible { opacity: 1; }
.remove { transition: opacity 150ms ease, background-color 150ms ease; }
.none { margin: 0; font-size: 12px; color: var(--muted); }
.none.center { padding: 28px 0; text-align: center; }
.add { align-self: flex-start; gap: 5px; }
.foot-note { margin: auto 0 0; padding-top: 10px; font-size: 11px; line-height: 15px; color: var(--muted); }
.chip { flex: none; display: inline-flex; align-items: center; height: 18px; padding: 0 7px; border-radius: 999px; background: var(--subtle); color: var(--muted); font-size: 11px; font-weight: 500; white-space: nowrap; }
.chip.back { background: var(--back-soft); color: var(--back); }
.chip.run { background: var(--info-soft); color: var(--info); }
.chip.warn { background: var(--warn-soft); color: var(--warn); }
.chip.bad { background: var(--danger-soft); color: var(--danger); }

.adder {
  position: sticky; bottom: 0; margin: 0 -8px; padding: 14px; border-radius: 14px; display: flex; flex-direction: column; gap: 10px;
  background: var(--glass-pop); -webkit-backdrop-filter: blur(40px) saturate(190%); backdrop-filter: blur(40px) saturate(190%); box-shadow: var(--pop-shadow);
}
.adder-hits { margin: 0; font-size: 12px; line-height: 17px; color: var(--label); }
.adder-buttons { display: flex; justify-content: flex-end; gap: 8px; }
.pop-enter-active { transition: opacity 180ms var(--ease), transform 180ms var(--ease); }
.pop-leave-active { transition: opacity 140ms cubic-bezier(0.4, 0, 1, 1), transform 140ms cubic-bezier(0.4, 0, 1, 1); }
.pop-enter-from, .pop-leave-to { opacity: 0; transform: translateY(6px); }

.main { flex: 1; min-width: 0; display: flex; flex-direction: column; }
.bar { flex: none; display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding: 10px 14px; }
.tabs { display: inline-flex; gap: 2px; padding: 2px; border-radius: 999px; background: var(--subtle); }
.tab { height: 24px; padding: 0 11px; border: 0; border-radius: 999px; background: transparent; font-size: 12px; display: inline-flex; gap: 5px; align-items: center; transition: background-color 180ms var(--ease), box-shadow 180ms var(--ease); }
.tab:hover { background: var(--hover); }
.tab.on { background: var(--seg-on); box-shadow: var(--seg-on-shadow); font-weight: 500; }
.tab:focus-visible { outline: none; box-shadow: 0 0 0 3px rgba(10, 132, 255, 0.42); }
.count { font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; }
.search { margin-left: auto; box-sizing: border-box; width: 220px; height: 24px; padding: 0 10px; border: 0; border-radius: 8px; background: var(--field-bg); box-shadow: inset 0 0 0 0.5px var(--field); font-size: 12px; outline: none; transition: box-shadow 150ms ease; }
.search:focus { box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.note { margin: 0; padding: 9px 11px; border-radius: 10px; font-size: 12px; line-height: 16px; }
.note.bad { background: var(--danger-soft); color: var(--danger); }
.note.inline { margin: 0 14px 10px; }

.table { flex: 1; min-height: 0; display: flex; flex-direction: column; transition: opacity 150ms ease; }
.table[aria-busy='true'] { opacity: 0.7; }
.tr { display: grid; grid-template-columns: 26px 44px 86px minmax(0, 1fr) 104px; align-items: center; column-gap: 10px; padding: 0 14px; min-height: 30px; border-top: 0.5px solid var(--line-soft); transition: background-color 150ms ease, color 180ms ease; }
.tr:not(.th):hover { background: var(--hover); }
.th { flex: none; min-height: 28px; background: var(--subtle); border-top: 0; font-size: 11px; font-weight: 600; color: var(--muted); }
.rows { flex: 1; min-height: 0; overflow-y: auto; overscroll-behavior: contain; display: flex; flex-direction: column; }
.tr.off { color: var(--muted); }
.tr.off .desc-text { text-decoration: line-through; text-decoration-color: var(--line); }
.tr.hit, .tr.hit:hover { background: var(--warn-soft); }
.line { font-size: 11px; color: var(--muted); }
.date, .num { font-variant-numeric: tabular-nums; }
.num { text-align: right; }
.amount { font-weight: 500; }
.tr.off .amount { font-weight: 400; }
.desc { min-width: 0; display: flex; align-items: center; gap: 8px; }
.desc-text { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.box { width: 16px; height: 16px; padding: 0; border: 0; border-radius: 5px; background: transparent; box-shadow: inset 0 0 0 1.25px var(--field); color: #fff; display: inline-flex; align-items: center; justify-content: center; transition: background-color 150ms ease, box-shadow 150ms ease, opacity 150ms ease; }
.box.on { background: var(--accent); box-shadow: none; }
.box:hover:not(:disabled):not(.on) { box-shadow: inset 0 0 0 1.25px var(--accent); }
.box:focus-visible { outline: none; box-shadow: 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.box:disabled { opacity: 0.45; }
.dash { width: 8px; height: 2px; border-radius: 1px; background: #fff; }
.more { align-self: center; margin: 12px 0 16px; }

.foot { flex: none; padding: 0 20px; border-top: 0.5px solid var(--line-soft); }
.foot-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 12px 0; }
.summary { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
.summary strong { font-weight: 600; }
.summary small { font-size: 11px; color: var(--muted); }
.actions { margin-left: auto; display: flex; gap: 8px; }
.give-back { gap: 6px; }
.approve { min-width: 110px; }

@container sheet (max-width: 860px) {
  .body { flex-direction: column; overflow-y: auto; }
  .side { width: auto; border-right: 0; border-bottom: 0.5px solid var(--line-soft); overflow: visible; }
  .main { min-height: 320px; }
}

@keyframes drop { from { opacity: 0; transform: translateY(-10px) scale(0.98); } to { opacity: 1; transform: none; } }
@keyframes lift { to { opacity: 0; transform: translateY(-8px) scale(0.98); } }
@keyframes fade { from { opacity: 0; } to { opacity: 1; } }
@keyframes fade-out { to { opacity: 0; } }
@media (prefers-reduced-motion: reduce) {
  .sheet { animation: fade 150ms ease both; }
  .modal-leave-active .sheet { animation: fade-out 140ms cubic-bezier(0.4, 0, 1, 1) both; }
  .pop-enter-from, .pop-leave-to { transform: none; }
}
</style>
