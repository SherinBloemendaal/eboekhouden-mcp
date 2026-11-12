import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { reactive } from 'vue'
import { inApp } from './server'

/** `waiting`: the call has answered, its change waits for the user's approval in EB MCP (Akkoord). */
export type Outcome = 'running' | 'waiting' | 'ok' | 'error' | 'refused'

export interface McpEntry {
  kind: 'mcp'
  id: number
  at: number
  client: string
  tool: string
  args: unknown
  account: string | null
  outcome: Outcome
  /**
   * The whole span from the call coming in to its end, the wait for the user's approval included; 0 until it ends.
   * This is what it always meant, so older entries and exports read the same. What EB MCP itself worked on the
   * call is `durationMs - waitedMs` (`workMs`).
   */
  durationMs: number
  bytes: number
  error: string | null
  calls: number[]
  result?: unknown
  /** The proposal on Akkoord of the change this call asked for, if it asked for one. */
  proposal?: number
  /** The EB MCP version that made the entry; empty for entries from before versions were recorded. */
  version: string
  /** When the change went up for approval (ms); null when it never waited. */
  waitStartedAt: number | null
  /** When the wait ended: approved, refused or lapsed (ms); null when it never waited or still waits. */
  waitEndedAt: number | null
  /** The part of `durationMs` spent waiting for the user; 0 when it never waited or still waits. */
  waitedMs: number
}

export interface EbEntry {
  kind: 'eb'
  id: number
  at: number
  parent: number | null
  client: string
  account: string | null
  method: string
  path: string
  query: [string, string][]
  status: number
  durationMs: number
  bytes: number
  rows: number | null
  error: string | null
  headers: [string, string][]
  response?: unknown
  truncated: boolean
  /** The EB MCP version that made the entry; empty for entries from before versions were recorded. */
  version: string
}

export type Entry = McpEntry | EbEntry

export interface Minute { eb: number; mcp: number; errors: number }
export interface Named { name: string; count: number; eb: number }
export interface EndpointStats { path: string; count: number; avgMs: number; p95Ms: number; errors: number; bytes: number }
export interface StatsView {
  startedAt: number
  mcp: number
  eb: number
  errors: number
  bytesIn: number
  bytesOut: number
  avgMs: number
  p95Ms: number
  minutes: [number, Minute][]
  endpoints: EndpointStats[]
  tools: Named[]
  clients: Named[]
  accounts: Named[]
  statuses: Named[]
}

export const log = reactive({ entries: [] as Entry[], loaded: false, paused: false, pending: 0 })

let timer: ReturnType<typeof setTimeout> | null = null
let listening = false

/** Refreshes at most twice a second, however busy the server is. */
function schedule(): void {
  if (log.paused) {
    log.pending += 1
    return
  }
  if (timer) return
  timer = setTimeout(async () => {
    timer = null
    await refreshLog()
  }, 500)
}

export async function refreshLog(): Promise<void> {
  log.entries = inApp() ? await invoke<Entry[]>('log_list') : []
  log.pending = 0
  log.loaded = true
}

export async function connectLog(onChange?: () => void): Promise<void> {
  await refreshLog()
  if (!inApp() || listening) return
  listening = true
  await listen<{ type: string }>('state', (event) => {
    if (event.payload.type === 'logChanged') {
      schedule()
      onChange?.()
    }
  })
}

export const entryDetail = (id: number) => invoke<Entry | null>('log_get', { id })
export const clearLog = () => invoke('log_clear').then(refreshLog)
/** Writes a debug export of the request log to a file; resolves to its absolute path, rejects with a Dutch sentence. */
export const exportLog = () => invoke<string>('log_export')
export const loadStats = () =>
  inApp()
    ? invoke<StatsView>('stats')
    : Promise.resolve<StatsView>({
        startedAt: Date.now(), mcp: 0, eb: 0, errors: 0, bytesIn: 0, bytesOut: 0, avgMs: 0, p95Ms: 0,
        minutes: [], endpoints: [], tools: [], clients: [], accounts: [], statuses: [],
      })

export function setPaused(paused: boolean): void {
  log.paused = paused
  if (!paused) void refreshLog()
}

export function duration(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(2).replace('.', ',')} s`
}

export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1).replace('.', ',')} kB`
  return `${(bytes / 1024 / 1024).toFixed(1).replace('.', ',')} MB`
}

export function time(ms: number): string {
  const d = new Date(ms)
  return `${d.toLocaleTimeString('nl-NL', { hour12: false })}.${String(d.getMilliseconds()).padStart(3, '0')}`
}

export function isError(entry: Entry): boolean {
  return entry.kind === 'mcp' ? entry.outcome === 'error' || entry.outcome === 'refused' : !!entry.error || entry.status >= 400 || entry.status === 0
}

export function statusLabel(entry: Entry): string {
  if (entry.kind === 'mcp') {
    return { running: 'Bezig', waiting: 'Wacht op akkoord', ok: 'OK', error: 'Fout', refused: 'Geweigerd' }[entry.outcome]
  }
  return entry.status ? String(entry.status) : 'Niet verstuurd'
}

/** The three parts of a call whose change waited for approval: preparing, waiting, making the change. */
export type Phase = 'prep' | 'wait' | 'exec'

export interface Phases {
  /** From the call coming in until the proposal went up. */
  prepMs: number
  /** Waiting for the user's answer; still counting while it waits. */
  waitMs: number
  /** From the approval to the end (or to now while it runs); 0 when refused or lapsed. */
  execMs: number
  totalMs: number
  /** What EB MCP itself worked: preparing plus making the change. */
  workMs: number
  /** The proposal still waits for an answer. */
  waiting: boolean
}

/** The phases of an MCP call that waited for approval, measured up to `now` while it waits or runs; null when it never waited. */
export function phases(entry: McpEntry, now = Date.now()): Phases | null {
  const started = entry.waitStartedAt ?? null
  if (started === null) return null
  const prepMs = Math.max(0, started - entry.at)
  const ended = entry.waitEndedAt ?? null
  if (ended === null) {
    const waitMs = Math.max(0, now - started)
    return { prepMs, waitMs, execMs: 0, totalMs: prepMs + waitMs, workMs: prepMs, waiting: true }
  }
  const waitMs = Math.max(0, ended - started)
  const totalMs = Math.max(prepMs + waitMs, entry.outcome === 'running' ? now - entry.at : entry.durationMs)
  const execMs = totalMs - prepMs - waitMs
  return { prepMs, waitMs, execMs, totalMs, workMs: prepMs + execMs, waiting: false }
}

/** What EB MCP itself worked on an entry: an MCP call without its wait for approval, an e-Boekhouden call as it took. */
export function workMs(entry: Entry, now = Date.now()): number {
  if (entry.kind === 'eb') return entry.durationMs
  return phases(entry, now)?.workMs ?? entry.durationMs
}

/** The phase an e-Boekhouden call of a waiting MCP call belongs to, by its own start against the wait's begin. */
export function phaseOf(call: EbEntry, entry: McpEntry): Exclude<Phase, 'wait'> {
  const started = entry.waitStartedAt ?? null
  return started === null || call.at <= started ? 'prep' : 'exec'
}

/** Where a moment lies on an MCP call's work time, counted from its start with the wait for approval taken out. */
export function workOffset(entry: McpEntry, at: number): number {
  const started = entry.waitStartedAt ?? null
  let offset = at - entry.at
  if (started !== null && at > started) offset -= Math.min(at, entry.waitEndedAt ?? at) - started
  return Math.max(0, offset)
}

/** How long a wait was: "47,3 s", "4 min 12 s", "1 u 5 min". */
export function waited(ms: number): string {
  if (ms < 60_000) return `${(Math.floor(ms / 100) / 10).toFixed(1).replace('.', ',')} s`
  const minutes = Math.floor(ms / 60_000)
  if (minutes < 60) {
    const seconds = Math.floor((ms % 60_000) / 1000)
    return seconds ? `${minutes} min ${seconds} s` : `${minutes} min`
  }
  const rest = minutes % 60
  return rest ? `${Math.floor(minutes / 60)} u ${rest} min` : `${Math.floor(minutes / 60)} u`
}

/** How long a wait has run so far, in steps that do not flicker: "12 s", "4 min", "1 u 5 min". */
export function waitingFor(ms: number): string {
  if (ms < 60_000) return `${Math.floor(ms / 1000)} s`
  const minutes = Math.floor(ms / 60_000)
  if (minutes < 60) return `${minutes} min`
  const rest = minutes % 60
  return rest ? `${Math.floor(minutes / 60)} u ${rest} min` : `${Math.floor(minutes / 60)} u`
}

/** The muted text next to the work time in Verzoeken: "+ 47,3 s gewacht", "wacht al 4 min"; empty without a wait. */
export function waitNote(entry: Entry, now = Date.now()): string {
  if (entry.kind !== 'mcp') return ''
  const p = phases(entry, now)
  if (!p) return ''
  return p.waiting ? `wacht al ${waitingFor(p.waitMs)}` : `+ ${waited(p.waitMs)} gewacht`
}

export interface Segment { phase: Phase; width: number }

/**
 * The parts of the duration bar in Verzoeken, as percentages of its track. `scale` is the work time that fills the
 * track (the longest work time listed), so one long wait never flattens the other rows: preparing and making the
 * change take their share of that scale, and the wait fills what is left of the track, at most. A call that never
 * waited has no segments: it keeps its one plain bar.
 */
export function segments(entry: Entry, scale: number, now = Date.now()): Segment[] {
  if (entry.kind !== 'mcp') return []
  const p = phases(entry, now)
  if (!p) return []
  const base = Math.max(1, scale)
  let prep = (p.prepMs / base) * 100
  let exec = (p.execMs / base) * 100
  // The work keeps the 4% the plain bar has at least, so it never vanishes next to the wait.
  const work = prep + exec
  if (work < 4) {
    if (work > 0) {
      prep = (prep / work) * 4
      exec = (exec / work) * 4
    } else prep = 4
  }
  if (prep + exec > 96) {
    const fit = 96 / (prep + exec)
    prep *= fit
    exec *= fit
  }
  const wait = Math.min(100 - prep - exec, Math.max(4, (p.waitMs / base) * 100))
  const parts: Segment[] = [{ phase: 'prep', width: prep }, { phase: 'wait', width: wait }]
  if (exec > 0) parts.push({ phase: 'exec', width: exec })
  return parts
}

/** The same parts in true proportion to the call's whole span, for the large bar in the detail. Each part shows at least a sliver. */
export function proportions(p: Phases): Segment[] {
  const total = Math.max(1, p.totalMs)
  const raw: Segment[] = [
    { phase: 'prep', width: (p.prepMs / total) * 100 },
    { phase: 'wait', width: (p.waitMs / total) * 100 },
    { phase: 'exec', width: (p.execMs / total) * 100 },
  ]
  const shown = raw.filter((s) => s.phase !== 'exec' || p.execMs > 0)
  const lifted = shown.map((s) => ({ ...s, width: Math.max(1.5, s.width) }))
  const sum = lifted.reduce((n, s) => n + s.width, 0)
  return lifted.map((s) => ({ ...s, width: (s.width / sum) * 100 }))
}

export const PHASE_LABELS: Record<Phase, string> = {
  prep: 'Voorbereiden',
  wait: 'Wacht op akkoord',
  exec: 'Uitvoeren in e-Boekhouden',
}

/** How the wait of a call ended, as the timeline names it. */
export interface WaitAnswer {
  decision: 'approved' | 'refused' | 'lapsed'
  /** Answered with Alles goedkeuren or Alles weigeren. */
  bulk: boolean
}

export interface Step {
  key: 'received' | 'prep' | 'answered' | 'wait' | 'approved' | 'refused' | 'lapsed' | 'exec' | 'done' | 'failed'
  /** The dot's colour: a phase, or how it ended. */
  tone: Phase | 'ok' | 'bad' | 'off'
  title: string
  /** A clock time or a duration, right of the title. */
  value: string
  note?: string
  chip?: string
  calls?: EbEntry[]
}

/**
 * The steps of an MCP call whose change waited for approval, top to bottom. `calls` are its e-Boekhouden calls;
 * `answer` is what the proposal's record says of the answer, when it was read (approval_get), so a bulk answer and
 * a refusal or lapse can be told apart; without it the log entry's own outcome decides. Empty for a call that never waited.
 */
export function waitSteps(entry: McpEntry, calls: EbEntry[], answer: WaitAnswer | null, now = Date.now()): Step[] {
  const p = phases(entry, now)
  const started = entry.waitStartedAt ?? null
  if (!p || started === null) return []
  const sorted = [...calls].sort((a, b) => a.at - b.at)
  const steps: Step[] = [
    { key: 'received', tone: 'prep', title: 'Aanroep ontvangen', value: time(entry.at) },
    { key: 'prep', tone: 'prep', title: 'Voorbereiden', value: duration(p.prepMs), calls: sorted.filter((c) => phaseOf(c, entry) === 'prep') },
    {
      key: 'answered', tone: 'wait', title: `Antwoord aan ${entry.client}`, value: time(started),
      note: entry.proposal !== undefined ? `Status wacht, voorstel ${entry.proposal}` : 'Status wacht',
    },
    { key: 'wait', tone: 'wait', title: 'Wacht op akkoord', value: p.waiting ? waitingFor(p.waitMs) : waited(p.waitMs), chip: 'niet meegeteld', note: p.waiting ? 'wacht nog' : undefined },
  ]
  const ended = entry.waitEndedAt ?? null
  if (p.waiting || ended === null) return steps
  // Without the record: a call that ended refused right at the end of its wait was never approved. The log's own
  // text for a proposal that lapsed at a restart says "vervallen".
  const unanswered = entry.outcome === 'refused' && p.execMs === 0
  const decision = answer?.decision ?? (unanswered ? (/vervallen/i.test(entry.error ?? '') ? 'lapsed' : 'refused') : 'approved')
  if (decision !== 'approved') {
    const lapsed = decision === 'lapsed'
    steps.push({
      key: lapsed ? 'lapsed' : 'refused', tone: 'off', title: lapsed ? 'Vervallen' : 'Geweigerd',
      value: time(ended) + (!lapsed && answer?.bulk ? ' · Alles weigeren' : ''), note: entry.error ?? undefined,
    })
    return steps
  }
  steps.push({ key: 'approved', tone: 'exec', title: 'Goedgekeurd', value: time(ended) + (answer?.bulk ? ' · Alles goedkeuren' : '') })
  const running = entry.outcome === 'running'
  steps.push({ key: 'exec', tone: 'exec', title: 'Uitvoeren', value: duration(p.execMs), note: running ? 'bezig' : undefined, calls: sorted.filter((c) => phaseOf(c, entry) === 'exec') })
  if (running) return steps
  const end = entry.at + p.totalMs
  if (entry.outcome === 'ok') steps.push({ key: 'done', tone: 'ok', title: 'Klaar', value: time(end), note: 'Uitgevoerd in e-Boekhouden' })
  else steps.push({ key: 'failed', tone: 'bad', title: 'Mislukt', value: time(end), note: entry.error ?? 'e-Boekhouden gaf geen reden' })
  return steps
}

/** e-Boekhouden calls in time order under their MCP call; the rest on their own. */
export function timeline(entries: Entry[]): { entry: Entry; children: EbEntry[] }[] {
  const byParent = new Map<number, EbEntry[]>()
  for (const e of entries) {
    if (e.kind === 'eb' && e.parent !== null) {
      const list = byParent.get(e.parent) ?? []
      list.push(e)
      byParent.set(e.parent, list)
    }
  }
  const mcpIds = new Set(entries.filter((e) => e.kind === 'mcp').map((e) => e.id))
  return entries
    .filter((e) => e.kind === 'mcp' || e.parent === null || !mcpIds.has(e.parent))
    .map((entry) => ({ entry, children: entry.kind === 'mcp' ? (byParent.get(entry.id) ?? []).sort((a, b) => a.at - b.at) : [] }))
}
