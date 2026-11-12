<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import EmptyState from '../components/EmptyState.vue'
import { connectLog, duration, loadStats, size, type Minute, type StatsView } from '../lib/log'

const LIMIT_PER_MINUTE = 60
const stats = ref<StatsView | null>(null)
const range = ref<15 | 60>(60)
const hover = ref<number | null>(null)
let timer: ReturnType<typeof setInterval> | null = null
let pending: ReturnType<typeof setTimeout> | null = null

async function load() {
  stats.value = await loadStats()
}

function soon() {
  if (pending) return
  pending = setTimeout(() => {
    pending = null
    void load()
  }, 1000)
}

onMounted(async () => {
  await load()
  await connectLog(soon)
  timer = setInterval(load, 10_000)
})
onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
  if (pending) clearTimeout(pending)
})

/** The last `range` minutes, empty minutes included, oldest first. */
const minutes = computed<{ at: number; m: Minute }[]>(() => {
  const now = Math.floor(Date.now() / 60_000)
  const map = new Map(stats.value?.minutes ?? [])
  return Array.from({ length: range.value }, (_, i) => {
    const at = now - range.value + 1 + i
    return { at, m: map.get(at) ?? { eb: 0, mcp: 0, errors: 0 } }
  })
})
const top = computed(() => Math.max(LIMIT_PER_MINUTE + 4, ...minutes.value.map((x) => x.m.eb + x.m.mcp + x.m.errors)))
const current = computed(() => minutes.value.at(-1)?.m ?? { eb: 0, mcp: 0, errors: 0 })
const spark = computed(() => {
  const pts = minutes.value.slice(-30).map((x) => x.m.eb + x.m.mcp)
  const max = Math.max(1, ...pts)
  return pts.map((v, i) => `${(i / (pts.length - 1)) * 96},${28 - (v / max) * 24}`).join(' ')
})
const total = computed(() => (stats.value ? stats.value.mcp + stats.value.eb : 0))
const errorRate = computed(() => (total.value ? (stats.value!.errors / total.value) * 100 : 0))
const recentErrors = computed(() => minutes.value.slice(-10).reduce((n, x) => n + x.m.errors, 0))
const p95Scale = computed(() => Math.max(1000, ...(stats.value?.endpoints ?? []).map((e) => e.p95Ms)))
const hovered = computed(() => (hover.value === null ? null : minutes.value[hover.value] ?? null))
const failing = computed(() => (stats.value?.endpoints ?? []).find((e) => e.errors >= 3 && e.errors / e.count >= 0.5) ?? null)

function clock(minute: number): string {
  return new Date(minute * 60_000).toLocaleTimeString('nl-NL', { hour: '2-digit', minute: '2-digit' })
}
/** "10 nov. 14:05": counting can start days before today, so the date is part of it. */
function since(ms: number): string {
  return new Date(ms).toLocaleString('nl-NL', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}
function pct(n: number): string {
  return `${n.toFixed(1).replace('.', ',').replace(',0', '')} %`
}
function bar(part: number, of: number): string {
  return `${of ? Math.max(1.5, (part / of) * 100) : 0}%`
}
</script>

<template>
  <EmptyState v-if="stats && total === 0" icon="chart" title="Nog geen aanroepen">
    Zodra een AI-client EB MCP gebruikt, zie je hier het verkeer: aanroepen per minuut, duur per endpoint, fouten en data. EB MCP bewaart het, ook na een herstart of update. Geteld vanaf {{ since(stats.startedAt) }}.
  </EmptyState>

  <div v-else-if="stats" class="page">
    <div v-if="failing" class="banner rise" role="status">
      <strong>{{ failing.path }} faalt vaak.</strong> {{ failing.errors }} van {{ failing.count }} aanroepen gaven een fout. Mogelijk is e-Boekhouden aangepast of traag.
      <RouterLink class="btn small" to="/verzoeken">Naar de verzoeken</RouterLink>
    </div>

    <div class="tiles">
      <div class="group tile">
        <span class="tile-label">Nu</span>
        <span class="tile-value"><span class="tile-number">{{ current.eb + current.mcp }}<span class="tile-unit">/min</span></span><svg class="spark" viewBox="0 0 96 30" preserveAspectRatio="none" aria-hidden="true"><polyline :points="spark" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" /></svg></span>
        <span class="tile-sub">e-Boekhouden {{ current.eb }} · MCP {{ current.mcp }}</span>
      </div>
      <div class="group tile">
        <span class="tile-label">Aanroepen</span>
        <span class="tile-value">{{ total.toLocaleString('nl-NL') }}</span>
        <span class="tile-sub">MCP {{ stats.mcp }} · e-Boekhouden {{ stats.eb }}</span>
      </div>
      <div class="group tile" :class="{ bad: recentErrors >= 3 }">
        <span class="tile-label">Foutpercentage</span>
        <span class="tile-value">{{ pct(errorRate) }}</span>
        <span class="tile-sub">{{ stats.errors }} van {{ total }}{{ recentErrors ? ` · ${recentErrors} in de laatste 10 min` : '' }}</span>
      </div>
      <div class="group tile">
        <span class="tile-label">Duur e-Boekhouden</span>
        <span class="tile-value">{{ duration(stats.avgMs) }}</span>
        <span class="tile-sub">gemiddeld · p95 {{ duration(stats.p95Ms) }}</span>
      </div>
      <div class="group tile">
        <span class="tile-label">Data</span>
        <span class="tile-value">{{ size(stats.bytesIn) }}</span>
        <span class="tile-sub">van e-Boekhouden · {{ size(stats.bytesOut) }} naar clients</span>
      </div>
    </div>

    <div class="grid">
      <section class="group card">
        <div class="card-head">
          <h2>Aanroepen per minuut</h2>
          <div class="seg" role="group" aria-label="Periode">
            <button type="button" :class="{ on: range === 15 }" :aria-pressed="range === 15" @click="range = 15">15 min</button>
            <button type="button" :class="{ on: range === 60 }" :aria-pressed="range === 60" @click="range = 60">60 min</button>
          </div>
        </div>
        <div class="legend">
          <span><i class="sw eb" />e-Boekhouden</span><span><i class="sw mcp" />MCP</span><span><i class="sw err" />Fouten</span><span><i class="sw limit" />Rem {{ LIMIT_PER_MINUTE }} per minuut</span>
        </div>
        <div class="chart" @mouseleave="hover = null">
          <span v-for="v in [0, 20, 40, 60]" :key="v" class="grid-line" :style="{ bottom: `${(v / top) * 100}%` }"><span>{{ v }}</span></span>
          <span class="limit" :style="{ bottom: `${(LIMIT_PER_MINUTE / top) * 100}%` }" />
          <div class="bars" role="img" :aria-label="`Aanroepen per minuut, laatste ${range} minuten`">
            <span v-for="(x, i) in minutes" :key="x.at" class="col" :class="{ dim: hover !== null && hover !== i }" @mouseenter="hover = i">
              <span class="seg-err" :style="{ height: `${(x.m.errors / top) * 100}%` }" />
              <span class="seg-mcp" :style="{ height: `${(x.m.mcp / top) * 100}%` }" />
              <span class="seg-eb" :style="{ height: `${(x.m.eb / top) * 100}%` }" />
            </span>
          </div>
          <div v-if="hovered" class="tip" role="tooltip" :style="{ left: `${((hover! + 0.5) / minutes.length) * 100}%` }">
            <strong>{{ clock(hovered.at) }}</strong><br />e-Boekhouden {{ hovered.m.eb }} · MCP {{ hovered.m.mcp }}<br />Fouten {{ hovered.m.errors }}
          </div>
        </div>
        <div class="axis"><span>{{ clock(minutes[0]!.at) }}</span><span>nu</span></div>
      </section>

      <div class="side">
        <section class="group card">
          <h2>MCP per client</h2>
          <p v-if="!stats.clients.length" class="muted">Verschijnt na de eerste aanroep.</p>
          <div v-for="c in stats.clients" :key="c.name" class="bar-row">
            <span class="bar-label">{{ c.name }}<small>{{ c.eb }} naar e-Boekhouden</small></span>
            <span class="track"><span class="fill mcp" :style="{ width: bar(c.count, Math.max(...stats.clients.map((x) => x.count))) }" /></span>
            <span class="num">{{ c.count }}</span>
          </div>
        </section>
        <section class="group card">
          <h2>Naar e-Boekhouden per account</h2>
          <p v-if="!stats.accounts.length" class="muted">Verschijnt na de eerste aanroep.</p>
          <div v-for="a in stats.accounts" :key="a.name" class="bar-row">
            <span class="bar-label">{{ a.name }}</span>
            <span class="track"><span class="fill eb" :style="{ width: bar(a.count, stats.accounts[0]!.count) }" /></span>
            <span class="num">{{ a.count }}</span>
          </div>
        </section>
      </div>

      <section class="group card">
        <div class="card-head"><h2>Per endpoint</h2><span class="muted small">Streep = gemiddeld, balk = p95</span></div>
        <div class="ep head"><span>Endpoint</span><span class="right">Aantal</span><span class="right">Gem.</span><span>p95</span><span class="right">Fouten</span><span class="right">Data</span></div>
        <div v-for="e in stats.endpoints" :key="e.path" class="ep" :class="{ bad: failing?.path === e.path }">
          <span class="mono path" :title="e.path">{{ e.path }}</span>
          <span class="right num">{{ e.count }}</span>
          <span class="right num">{{ duration(e.avgMs) }}</span>
          <span class="p95"><span class="track"><span class="p95-bar" :class="{ slow: e.p95Ms >= 1000 }" :style="{ width: `${(e.p95Ms / p95Scale) * 100}%` }" /><span class="avg" :style="{ left: `${(e.avgMs / p95Scale) * 100}%` }" /></span><span class="num">{{ duration(e.p95Ms) }}</span></span>
          <span class="right num" :class="{ badtext: e.errors }">{{ e.errors }}</span>
          <span class="right num muted">{{ size(e.bytes) }}</span>
        </div>
      </section>

      <div class="side">
        <section class="group card">
          <h2>Top tools</h2>
          <p v-if="!stats.tools.length" class="muted">Verschijnt na de eerste aanroep.</p>
          <div v-for="t in stats.tools.slice(0, 10)" :key="t.name" class="bar-row">
            <span class="bar-label mono">{{ t.name }}</span>
            <span class="track"><span class="fill mcp" :style="{ width: bar(t.count, stats.tools[0]!.count) }" /></span>
            <span class="num">{{ t.count }}</span>
          </div>
        </section>
        <section class="group card">
          <h2>Statuscodes</h2>
          <div class="status-bar" aria-hidden="true">
            <span v-for="s in stats.statuses" :key="s.name" :class="s.name.startsWith('2') ? 'ok' : s.name === 'Geweigerd' ? 'off' : s.name.startsWith('4') ? 'warn' : 'bad'" :style="{ width: bar(s.count, stats.statuses.reduce((n, x) => n + x.count, 0)) }" />
          </div>
          <div class="status-legend">
            <span v-for="s in stats.statuses" :key="s.name">{{ s.name }} <strong>{{ s.count }}</strong></span>
          </div>
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* The page is the container: the window can be made narrow, and tiles and cards follow its width, not the screen's. */
.page { padding: 8px 28px 28px; display: flex; flex-direction: column; gap: 16px; container: dash / inline-size; }
.banner { display: flex; align-items: center; flex-wrap: wrap; gap: 6px 10px; padding: 10px 14px; border-radius: 12px; background: var(--danger-soft); color: var(--danger); font-size: 12px; }
.banner .btn { margin-left: auto; text-decoration: none; color: var(--text); }
/* As many tiles in a row as fit at 150px each: five in a wide window, fewer rows of equal tiles in a narrow one. */
.tiles { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 12px; }
.tile { padding: 12px 14px; display: flex; flex-direction: column; gap: 3px; min-width: 0; transition: background-color 150ms ease; }
.tile.bad { background: var(--danger-soft); }
.tile.bad .tile-label, .tile.bad .tile-sub, .tile.bad .tile-value { color: var(--danger); }
.tile-label { font-size: 11px; color: var(--muted); }
.tile-value { display: flex; align-items: flex-end; justify-content: space-between; gap: 10px; min-width: 0; font-size: 24px; line-height: 30px; font-weight: 700; font-variant-numeric: tabular-nums; white-space: nowrap; }
.tile-number { flex: none; display: inline-flex; align-items: baseline; gap: 4px; }
.tile-unit { font-size: 13px; line-height: 18px; font-weight: 500; color: var(--muted); }
/* The line takes what the number leaves, up to its drawn width, and never pushes the number onto two lines. */
.spark { flex: 0 1 96px; min-width: 0; height: 30px; color: var(--accent); }
.tile-sub { font-size: 11px; line-height: 15px; color: var(--muted); overflow-wrap: anywhere; }
.grid { display: grid; grid-template-columns: minmax(0, 2fr) minmax(0, 1fr); gap: 16px; align-items: start; }
.side { display: flex; flex-direction: column; gap: 16px; min-width: 0; }
.card { padding: 14px 16px; display: flex; flex-direction: column; gap: 10px; min-width: 0; }
.card h2 { font-size: 13px; font-weight: 700; }
.card-head { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
.seg { display: inline-flex; padding: 2px; border-radius: 999px; background: var(--subtle); }
.seg button { height: 22px; padding: 0 10px; border: 0; border-radius: 999px; background: transparent; font-size: 12px; transition: background-color 180ms var(--ease); }
.seg button.on { background: var(--seg-on); box-shadow: var(--seg-on-shadow); font-weight: 500; }
.legend { display: flex; flex-wrap: wrap; gap: 12px; font-size: 11px; color: var(--label); }
.legend span { display: flex; align-items: center; gap: 5px; }
.sw { width: 9px; height: 9px; border-radius: 2px; display: inline-block; }
.sw.eb { background: #0a84ff; }
.sw.mcp { background: #af52de; }
.sw.err { background: #d93025; }
.sw.limit { width: 12px; height: 0; border-top: 2px dashed #ff9f0a; border-radius: 0; }
.chart { position: relative; height: 180px; margin-left: 24px; }
.grid-line { position: absolute; left: 0; right: 0; height: 1px; background: var(--line-soft); }
.grid-line span { position: absolute; left: -24px; top: -7px; width: 18px; text-align: right; font-size: 10px; color: var(--muted); }
/* Only the line in the chart: the legend's swatch has the same class and must stay in its row. */
.chart > .limit { position: absolute; left: 0; right: 0; border-top: 2px dashed #ff9f0a; opacity: 0.8; }
.bars { position: absolute; inset: 0; display: flex; align-items: flex-end; gap: 2px; }
.col { flex: 1; height: 100%; display: flex; flex-direction: column; justify-content: flex-end; transition: opacity 150ms ease; }
.col.dim { opacity: 0.5; }
.seg-eb { background: #0a84ff; }
.seg-mcp { background: #af52de; }
.seg-err { background: #d93025; border-radius: 2px 2px 0 0; }
.tip {
  position: absolute; bottom: calc(100% + 6px); transform: translateX(-50%); width: 170px; padding: 7px 9px; border-radius: 9px; font-size: 11px; line-height: 15px; pointer-events: none;
  background: var(--glass-pop); -webkit-backdrop-filter: blur(30px); backdrop-filter: blur(30px); box-shadow: var(--pop-shadow); animation: tip 150ms ease both; animation-delay: 300ms;
}
@keyframes tip { from { opacity: 0; transform: translate(-50%, 4px); } to { opacity: 1; transform: translateX(-50%); } }
.axis { margin-left: 24px; display: flex; justify-content: space-between; font-size: 10px; color: var(--muted); }
.bar-row { display: grid; grid-template-columns: minmax(0, 1.2fr) 1fr 40px; gap: 8px; align-items: center; font-size: 12px; }
.bar-label { display: flex; flex-direction: column; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.bar-label small { font-size: 10px; color: var(--muted); }
.track { position: relative; height: 6px; border-radius: 3px; background: var(--sunken); }
.fill { position: absolute; left: 0; top: 0; bottom: 0; border-radius: 3px; }
.fill.mcp { background: #af52de; }
.fill.eb { background: #0a84ff; }
.num { text-align: right; font-variant-numeric: tabular-nums; font-size: 12px; }
.ep { display: grid; grid-template-columns: minmax(0, 1fr) 54px 64px 140px 50px 64px; gap: 10px; align-items: center; min-height: 26px; padding: 0 6px; margin: 0 -6px; border-radius: 6px; transition: background-color 150ms ease; }
.ep:not(.head):hover { background: var(--hover); }
.ep.head { min-height: 22px; font-size: 11px; font-weight: 600; color: var(--label); }
.ep.bad { background: var(--danger-soft); }
.path { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-size: 12px; }
.right { text-align: right; }
.p95 { display: flex; align-items: center; gap: 6px; }
.p95 .track { flex: 1; height: 8px; }
.p95-bar { position: absolute; left: 0; top: 0; bottom: 0; border-radius: 4px; background: #0a84ff; opacity: 0.35; }
.p95-bar.slow { background: #ff9f0a; }
.avg { position: absolute; top: -2px; width: 3px; height: 12px; border-radius: 2px; background: var(--text); }
.p95 .num { width: 52px; }
.badtext { color: var(--danger); font-weight: 500; }
.muted { color: var(--muted); }
.small { font-size: 11px; }
.status-bar { display: flex; gap: 2px; height: 10px; border-radius: 5px; overflow: hidden; }
.status-bar .ok { background: #28a745; }
.status-bar .warn { background: #ff9f0a; }
.status-bar .bad { background: #d93025; }
.status-bar .off { background: var(--off); }
.status-legend { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: 12px; color: var(--label); }
@container dash (max-width: 760px) {
  .grid { grid-template-columns: minmax(0, 1fr); }
}
@media (prefers-reduced-motion: reduce) { .tip { animation: fade-in 150ms ease 300ms both; } }
</style>
