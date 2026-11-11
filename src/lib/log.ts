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
  durationMs: number
  bytes: number
  error: string | null
  calls: number[]
  result?: unknown
  /** The proposal on Akkoord of the change this call asked for, if it asked for one. */
  proposal?: number
  /** The EB MCP version that made the entry; empty for entries from before versions were recorded. */
  version: string
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
