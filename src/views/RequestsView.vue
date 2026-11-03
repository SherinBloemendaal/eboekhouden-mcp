<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import Alert from '../components/Alert.vue'
import EmptyState from '../components/EmptyState.vue'
import Icon from '../components/Icon.vue'
import JsonView from '../components/JsonView.vue'
import {
  clearLog, connectLog, duration, entryDetail, isError, log, setPaused, size, statusLabel, time, timeline,
  type EbEntry, type Entry,
} from '../lib/log'

type Tab = 'all' | 'mcp' | 'eb'

const tab = ref<Tab>('all')
const search = ref('')
const client = ref('')
const account = ref('')
const errorsOnly = ref(false)
const expanded = ref(new Set<number>())
const selectedId = ref<number | null>(null)
const detail = ref<Entry | null>(null)
const clearing = ref(false)

onMounted(() => connectLog())

const counts = computed(() => ({
  all: log.entries.length,
  mcp: log.entries.filter((e) => e.kind === 'mcp').length,
  eb: log.entries.filter((e) => e.kind === 'eb').length,
}))
const clients = computed(() => [...new Set(log.entries.map((e) => e.client))].sort())
const accounts = computed(() => [...new Set(log.entries.map((e) => e.account).filter((a): a is string => !!a))].sort())
const filtering = computed(() => !!(search.value || client.value || account.value || errorsOnly.value))

function matches(entry: Entry): boolean {
  if (client.value && entry.client !== client.value) return false
  if (account.value && entry.account !== account.value) return false
  if (errorsOnly.value && !isError(entry)) return false
  const term = search.value.trim().toLowerCase()
  if (term) {
    const text = entry.kind === 'mcp' ? `${entry.tool} ${JSON.stringify(entry.args)}` : entry.path
    if (!text.toLowerCase().includes(term)) return false
  }
  return true
}

const rows = computed(() => {
  if (tab.value === 'eb') return log.entries.filter((e) => e.kind === 'eb' && matches(e)).map((entry) => ({ entry, children: [] as EbEntry[] }))
  if (tab.value === 'mcp') return log.entries.filter((e) => e.kind === 'mcp' && matches(e)).map((entry) => ({ entry, children: [] as EbEntry[] }))
  return timeline(log.entries).filter((r) => matches(r.entry) || r.children.some(matches))
})

const maxMs = computed(() => Math.max(1000, ...log.entries.map((e) => e.durationMs)))

function toggle(id: number) {
  const next = new Set(expanded.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  expanded.value = next
}

async function select(entry: Entry) {
  selectedId.value = entry.id
  detail.value = await entryDetail(entry.id)
}

watch(
  () => log.entries,
  async () => {
    if (selectedId.value !== null) detail.value = await entryDetail(selectedId.value)
  },
)

const detailCalls = computed<EbEntry[]>(() => {
  const d = detail.value
  if (!d || d.kind !== 'mcp') return []
  return log.entries.filter((e): e is EbEntry => e.kind === 'eb' && e.parent === d.id).sort((a, b) => a.at - b.at)
})
const span = computed(() => {
  const calls = detailCalls.value
  const d = detail.value
  if (!d || !calls.length) return { start: 0, total: 1 }
  const start = d.at
  const end = Math.max(d.at + d.durationMs, ...calls.map((c) => c.at + c.durationMs))
  return { start, total: Math.max(1, end - start) }
})
const parentOf = computed(() => {
  const d = detail.value
  if (!d || d.kind !== 'eb' || d.parent === null) return null
  const parent = log.entries.find((e) => e.id === d.parent)
  if (!parent || parent.kind !== 'mcp') return null
  const index = parent.calls.indexOf(d.id) + 1
  return { parent, index, total: parent.calls.length }
})

function clearFilters() {
  search.value = ''
  client.value = ''
  account.value = ''
  errorsOnly.value = false
}

async function confirmClear() {
  await clearLog()
  clearing.value = false
  selectedId.value = null
  detail.value = null
}

function initials(text: string | null): string {
  if (!text) return ''
  const words = text.split(/\s+/)
  return ((words[0]?.[0] ?? '') + (words[1]?.[0] ?? words[0]?.[1] ?? '')).toUpperCase()
}
</script>

<template>
  <Teleport to="#toolbar-actions" defer>
    <button type="button" class="btn small danger-text" :disabled="!log.entries.length" @click="clearing = true"><Icon name="trash" :size="14" />Leegmaken</button>
  </Teleport>

  <div class="page">
    <div class="filters">
      <div class="tabs" role="tablist" aria-label="Bron">
        <button v-for="t in (['all', 'mcp', 'eb'] as Tab[])" :key="t" type="button" role="tab" :aria-selected="tab === t" class="tab" :class="{ on: tab === t }" @click="tab = t">
          {{ { all: 'Alles', mcp: 'MCP', eb: 'e-Boekhouden' }[t] }}<span class="count">{{ counts[t] }}</span>
        </button>
      </div>
      <input v-model="search" class="search" type="search" placeholder="Zoek op tool of pad" aria-label="Zoek op tool of pad" />
      <select v-model="client" class="pick" :class="{ on: client }" aria-label="Client">
        <option value="">Alle clients</option>
        <option v-for="c in clients" :key="c" :value="c">{{ c }}</option>
      </select>
      <select v-model="account" class="pick" :class="{ on: account }" aria-label="Account">
        <option value="">Alle accounts</option>
        <option v-for="a in accounts" :key="a" :value="a">{{ a }}</option>
      </select>
      <button type="button" class="pick" :class="{ on: errorsOnly }" :aria-pressed="errorsOnly" @click="errorsOnly = !errorsOnly">Alleen fouten</button>
      <button type="button" class="btn small live" :aria-pressed="!log.paused" @click="setPaused(!log.paused)">
        <template v-if="log.paused">Gepauzeerd{{ log.pending ? ` · ${log.pending} nieuw` : '' }}</template>
        <template v-else><span class="dot on" />Live</template>
      </button>
    </div>

    <EmptyState v-if="log.loaded && !log.entries.length" icon="inbox" title="Nog geen verzoeken">
      Elke MCP-aanroep en de aanroepen naar e-Boekhouden die hij veroorzaakt, verschijnen hier. Alleen in het geheugen, de laatste 1000.
    </EmptyState>
    <EmptyState v-else-if="log.loaded && !rows.length" icon="list" title="Geen verzoeken voor deze filters">
      Pas de zoekterm of de filters aan.
      <template #actions><button type="button" class="btn" @click="clearFilters">Filters wissen</button></template>
    </EmptyState>

    <div v-else class="split">
      <div class="list" role="table" aria-label="Verzoeken">
        <div class="row head" role="row"><span>Tijd</span><span>Bron</span><span>Tool of pad</span><span>Status</span><span class="right">Duur</span><span class="right">Grootte</span><span>Wie</span></div>
        <div class="body">
          <template v-for="{ entry, children } in rows" :key="entry.id">
            <div class="row item" role="row" :class="{ sel: selectedId === entry.id }" @click="select(entry)">
              <span class="mono muted small">{{ time(entry.at) }}</span>
              <span><span class="badge" :class="entry.kind">{{ entry.kind === 'mcp' ? 'MCP' : (entry as EbEntry).method }}</span></span>
              <span class="main">
                <template v-if="entry.kind === 'mcp'">
                  <button v-if="tab === 'all' && children.length" type="button" class="chev" :aria-expanded="expanded.has(entry.id)" aria-label="Aanroepen naar e-Boekhouden tonen" @click.stop="toggle(entry.id)">
                    <span :class="{ open: expanded.has(entry.id) }">›</span>
                  </button>
                  <span v-else class="chev-space" />
                  <strong class="mono">{{ entry.tool }}</strong>
                  <span class="mono muted args">{{ JSON.stringify(entry.args) }}</span>
                  <span v-if="tab === 'all'" class="chip">{{ entry.calls.length === 1 ? '1 aanroep' : `${entry.calls.length} aanroepen` }}</span>
                </template>
                <span v-else class="mono path">{{ entry.path }}</span>
              </span>
              <span class="status" :class="{ bad: isError(entry), warn: entry.kind === 'mcp' && entry.outcome === 'refused' }"><span class="dot" :class="isError(entry) ? 'bad' : 'on-static'" />{{ statusLabel(entry) }}</span>
              <span class="right dur">{{ duration(entry.durationMs) }}<span class="bar"><span :style="{ width: `${Math.max(4, (entry.durationMs / maxMs) * 100)}%` }" :class="{ slow: entry.durationMs >= 1000 }" /></span></span>
              <span class="right muted">{{ size(entry.bytes) }}</span>
              <span class="who" :title="`${entry.client}${entry.account ? ` · ${entry.account}` : ''}`"><span class="tag">{{ initials(entry.client) }}</span><span v-if="entry.account" class="tag acc">{{ initials(entry.account) }}</span></span>
            </div>
            <template v-if="tab === 'all' && expanded.has(entry.id)">
              <div v-for="(child, i) in children" :key="child.id" class="row item child" role="row" :class="{ sel: selectedId === child.id }" @click="select(child)">
                <span class="mono muted small">{{ time(child.at) }}</span>
                <span><span class="badge eb">{{ child.method }}</span></span>
                <span class="main"><span class="tree" :class="{ last: i === children.length - 1 }" /><span class="mono path">{{ child.path }}</span></span>
                <span class="status" :class="{ bad: isError(child) }"><span class="dot" :class="isError(child) ? 'bad' : 'on-static'" />{{ statusLabel(child) }}</span>
                <span class="right dur">{{ duration(child.durationMs) }}<span class="bar"><span :style="{ width: `${Math.max(4, (child.durationMs / maxMs) * 100)}%` }" :class="{ slow: child.durationMs >= 1000 }" /></span></span>
                <span class="right muted">{{ size(child.bytes) }}</span>
                <span class="who"><span class="tag">{{ initials(child.client) }}</span><span v-if="child.account" class="tag acc">{{ initials(child.account) }}</span></span>
              </div>
            </template>
          </template>
        </div>
      </div>

      <aside v-if="detail" class="detail" aria-label="Details">
        <div class="detail-head">
          <div class="title-line">
            <span class="badge" :class="detail.kind">{{ detail.kind === 'mcp' ? 'MCP' : detail.method }}</span>
            <strong class="mono title">{{ detail.kind === 'mcp' ? detail.tool : detail.path }}</strong>
            <button type="button" class="icon-btn" aria-label="Details sluiten" title="Sluiten" @click="detail = null; selectedId = null">✕</button>
          </div>
          <div class="facts">
            <span class="chip" :class="isError(detail) ? 'bad' : 'ok'">{{ statusLabel(detail) }}</span>
            <span class="chip">{{ duration(detail.durationMs) }}</span>
            <span class="chip">{{ size(detail.bytes) }}</span>
            <span class="chip">{{ detail.client }}</span>
            <span v-if="detail.account" class="chip info">{{ detail.account }}</span>
            <span class="chip mono">{{ time(detail.at) }}</span>
          </div>
          <button v-if="parentOf" type="button" class="parent" @click="select(parentOf.parent)">
            Door <strong class="mono">{{ parentOf.parent.tool }}</strong> van {{ parentOf.parent.client }}, aanroep {{ parentOf.index }} van {{ parentOf.total }}
          </button>
        </div>
        <div class="detail-body">
          <p v-if="detail.error" class="note bad" role="alert">{{ detail.error }}</p>
          <template v-if="detail.kind === 'mcp'">
            <section><h3 class="section-label">Argumenten</h3><JsonView :value="detail.args" /></section>
            <section v-if="detailCalls.length">
              <h3 class="section-label">Naar e-Boekhouden · {{ detailCalls.length }} {{ detailCalls.length === 1 ? 'aanroep' : 'aanroepen' }}, {{ duration(detail.durationMs) }}</h3>
              <button v-for="call in detailCalls" :key="call.id" type="button" class="wf" @click="select(call)">
                <span class="mono path">{{ call.path }}</span>
                <span class="wf-track"><span class="wf-bar" :class="{ bad: isError(call) }" :style="{ left: `${((call.at - span.start) / span.total) * 100}%`, width: `${Math.max(2, (call.durationMs / span.total) * 100)}%` }" /></span>
                <span class="right small">{{ duration(call.durationMs) }}</span>
              </button>
            </section>
            <section v-if="detail.result !== undefined && detail.result !== null"><h3 class="section-label">Resultaat, zoals de client het kreeg</h3><JsonView :value="detail.result" :max-height="320" /></section>
          </template>
          <template v-else>
            <section v-if="detail.query.length">
              <h3 class="section-label">Query</h3>
              <dl class="kv"><template v-for="[k, v] in detail.query" :key="k"><dt class="mono">{{ k }}</dt><dd class="mono selectable">{{ v || '(leeg)' }}</dd></template></dl>
            </section>
            <section>
              <h3 class="section-label">Headers</h3>
              <dl class="kv"><template v-for="[k, v] in detail.headers" :key="k"><dt class="mono">{{ k }}</dt><dd class="mono">{{ v }}<span v-if="v.startsWith('••')" class="chip">verborgen</span></dd></template></dl>
            </section>
            <section v-if="detail.response !== undefined && detail.response !== null">
              <h3 class="section-label">Antwoord<span v-if="detail.rows !== null" class="chip">{{ detail.rows }} rijen</span><span v-if="detail.truncated" class="chip">ingekort</span></h3>
              <JsonView :value="detail.response" :max-height="360" />
            </section>
          </template>
        </div>
      </aside>
    </div>
  </div>

  <Alert v-if="clearing" :title="`${log.entries.length} verzoeken leegmaken?`" tone="danger" @close="clearing = false">
    <p>De log staat alleen in het geheugen. Leegmaken haalt alle aanroepen weg en kan niet terug. Het dashboard telt door.</p>
    <template #buttons>
      <button type="button" class="btn" @click="clearing = false">Annuleren</button>
      <button type="button" class="btn danger-fill" @click="confirmClear">Leegmaken</button>
    </template>
  </Alert>
</template>

<style scoped>
.page { height: 100%; display: flex; flex-direction: column; }
.filters { flex: none; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 2px 16px 10px; }
.tabs { display: inline-flex; gap: 2px; padding: 2px; border-radius: 999px; background: var(--subtle); }
.tab { height: 24px; padding: 0 11px; border: 0; border-radius: 999px; background: transparent; font-size: 12px; display: inline-flex; gap: 5px; align-items: center; transition: background-color 180ms var(--ease), box-shadow 180ms var(--ease); }
.tab:hover { background: var(--hover); }
.tab.on { background: var(--seg-on); box-shadow: var(--seg-on-shadow); font-weight: 500; }
.count { font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; }
.search { box-sizing: border-box; width: 200px; height: 24px; padding: 0 10px; border: 0; border-radius: 8px; background: var(--field-bg); box-shadow: inset 0 0 0 0.5px var(--field); font-size: 12px; outline: none; -webkit-user-select: text; user-select: text; }
.search:focus { box-shadow: inset 0 0 0 1px var(--accent), 0 0 0 3.5px rgba(10, 132, 255, 0.42); }
.pick { height: 24px; padding: 0 10px; border: 0; border-radius: 999px; background: var(--glass-bar); box-shadow: var(--glass-bar-edge); font-size: 12px; transition: background-color 150ms ease; }
.pick.on { background: var(--info-soft); color: var(--info); box-shadow: inset 0 0 0 0.5px var(--accent); }
.live { margin-left: auto; }
.split { flex: 1; min-height: 0; display: flex; gap: 0; }
.list { flex: 1; min-width: 0; display: flex; flex-direction: column; }
.body { flex: 1; overflow: auto; padding: 4px 0; }
.row { display: grid; grid-template-columns: 92px 54px minmax(0, 1fr) 84px 96px 62px 52px; align-items: center; column-gap: 10px; }
.row.head { height: 26px; padding: 0 16px; background: var(--sunken); font-size: 11px; font-weight: 600; color: var(--label); }
.row.item { height: 28px; margin: 0 8px; padding: 0 8px; border-radius: 6px; transition: background-color 150ms ease; }
.row.item:hover { background: var(--hover); }
.row.item.sel { background: var(--info-soft); }
.row.child .main { padding-left: 4px; }
.right { text-align: right; justify-content: flex-end; }
.muted { color: var(--muted); }
.small { font-size: 11px; }
.badge { display: inline-flex; align-items: center; height: 18px; padding: 0 7px; border-radius: 999px; font-size: 11px; font-weight: 500; }
.badge.mcp { background: rgba(175, 82, 222, 0.13); color: #7a3fc7; }
.badge.eb { background: var(--info-soft); color: var(--info); font-family: var(--mono); }
.main { display: flex; align-items: center; gap: 8px; min-width: 0; }
.args, .path { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; flex: 1; }
.args { font-size: 11px; }
.chev, .chev-space { width: 18px; height: 18px; flex: none; }
.chev { border: 0; border-radius: 5px; background: transparent; color: var(--muted); padding: 0; }
.chev:hover { background: var(--hover); }
.chev span { display: inline-block; transition: transform 180ms var(--ease); }
.chev span.open { transform: rotate(90deg); }
.tree { position: relative; width: 18px; height: 28px; flex: none; }
.tree::before { content: ''; position: absolute; left: 8px; top: 0; bottom: 0; width: 1px; background: var(--line); }
.tree.last::before { bottom: 50%; }
.tree::after { content: ''; position: absolute; left: 8px; top: 50%; width: 9px; height: 1px; background: var(--line); }
.chip { display: inline-flex; align-items: center; height: 18px; padding: 0 7px; border-radius: 999px; background: var(--subtle); color: var(--label); font-size: 11px; font-weight: 500; white-space: nowrap; margin-left: 6px; }
.chip.ok { background: var(--success-soft); color: var(--success); margin-left: 0; }
.chip.bad { background: var(--danger-soft); color: var(--danger); margin-left: 0; }
.chip.info { background: var(--info-soft); color: var(--info); }
.status { display: flex; align-items: center; gap: 6px; font-size: 12px; font-weight: 500; color: var(--success); }
.status.bad { color: var(--danger); }
.status.warn { color: var(--warn); }
.dot.on-static { background: var(--success-solid); }
.dur { display: flex; align-items: center; gap: 6px; font-size: 12px; font-variant-numeric: tabular-nums; }
.bar { width: 32px; height: 4px; border-radius: 2px; background: var(--line-soft); overflow: hidden; flex: none; }
.bar span { display: block; height: 4px; background: #0a84ff; }
.bar span.slow { background: #ff9f0a; }
.who { display: flex; gap: 3px; }
.tag { height: 18px; min-width: 20px; padding: 0 3px; box-sizing: border-box; border-radius: 5px; background: var(--subtle); color: var(--label); font-size: 9px; font-weight: 700; display: inline-flex; align-items: center; justify-content: center; }
.tag.acc { border-radius: 50%; background: var(--info-soft); color: var(--info); }
.detail {
  width: 400px; flex: none; margin: 0 8px 8px 0; border-radius: 14px; overflow: hidden; display: flex; flex-direction: column;
  background: var(--group); box-shadow: var(--group-edge); animation: slide 240ms var(--ease) both;
}
@keyframes slide { from { opacity: 0; transform: translateX(12px); } to { opacity: 1; transform: none; } }
.detail-head { padding: 14px 16px 10px; border-bottom: 0.5px solid var(--line-soft); display: flex; flex-direction: column; gap: 8px; }
.title-line { display: flex; align-items: center; gap: 8px; }
.title { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-size: 13px; }
.facts { display: flex; flex-wrap: wrap; gap: 6px; }
.facts .chip { margin-left: 0; }
.parent { text-align: left; border: 0; border-radius: 10px; padding: 7px 10px; background: rgba(175, 82, 222, 0.13); color: #7a3fc7; font-size: 12px; }
.detail-body { flex: 1; overflow: auto; padding: 12px 16px 18px; display: flex; flex-direction: column; gap: 16px; }
section { display: flex; flex-direction: column; gap: 6px; }
h3 { display: flex; align-items: center; margin: 0; }
.kv { margin: 0; display: grid; grid-template-columns: 110px 1fr; }
.kv dt, .kv dd { margin: 0; padding: 4px 0; border-top: 0.5px solid var(--line-soft); font-size: 12px; }
.kv dt { color: var(--muted); }
.kv dd { word-break: break-all; }
.wf { display: grid; grid-template-columns: minmax(0, 1fr) 1fr 58px; gap: 8px; align-items: center; height: 28px; padding: 0 6px; border: 0; border-radius: 8px; background: transparent; text-align: left; transition: background-color 120ms ease; }
.wf:hover { background: var(--hover); }
.wf .path { font-size: 11px; }
.wf-track { position: relative; height: 8px; border-radius: 3px; background: var(--sunken); }
.wf-bar { position: absolute; top: 0; bottom: 0; border-radius: 3px; background: #0a84ff; }
.wf-bar.bad { background: #d93025; }
.note { padding: 9px 11px; border-radius: 10px; font-size: 12px; line-height: 16px; }
.note.bad { background: var(--danger-soft); color: var(--danger); }
.icon-btn { width: 24px; height: 24px; border: 0; border-radius: 7px; background: transparent; color: var(--muted); transition: background-color 150ms ease; }
.icon-btn:hover { background: var(--hover); color: var(--text); }
@media (max-width: 1240px) { .detail { width: 340px; } .row { grid-template-columns: 84px 50px minmax(0, 1fr) 76px 92px 52px; } .row > :nth-child(6) { display: none; } }
@media (prefers-reduced-motion: reduce) { .detail { animation: none; } .chev span { transition: none; } }
</style>
