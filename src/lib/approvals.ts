import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { reactive } from 'vue'
import { since, store } from './accounts'
import { inApp } from './server'

/**
 * A change an AI client asked for in an account set to "met akkoord" (approvals.rs). It waits in a queue; the client
 * goes on, and EB MCP makes the change in the background once the user approves it.
 */
export interface Proposal {
  id: number
  accountId: string
  account: string
  client: string
  tool: string
  title: string
  lines: { label: string; value: string }[]
  /** Cannot be undone: asked even for an account that needs no approval otherwise. */
  danger: boolean
  askedAt: number
}

export type ApprovalStatus = 'waiting' | 'running' | 'done' | 'failed' | 'refused' | 'lapsed'
/** How the proposal was answered: by the user, without approval (the account books freely), or not in time. */
export type ApprovalDecision = 'approved' | 'refused' | 'auto' | 'lapsed'

/** A proposal in the history, waiting or answered, with what came of it. */
export interface ApprovalRecord extends Proposal {
  decidedAt: number | null
  decision: ApprovalDecision | null
  status: ApprovalStatus
  error: string | null
  /** The EB MCP version that took the proposal in. */
  version: string
  /** Answered with Alles goedkeuren or Alles weigeren rather than on its own. */
  bulk: boolean
}

/** A record with what EB MCP got from the client, sent to e-Boekhouden and got back. */
export interface ApprovalDetail extends ApprovalRecord {
  args: unknown
  body: unknown | null
  result: unknown | null
}

export interface DecideAllResult {
  decided: number[]
  /** Ids that no longer waited: answered elsewhere, or lapsed. */
  gone: number[]
}

/** What this window knows besides the queue itself, which lives in `store.approvals`. */
export const approvalState = reactive({
  /** True once the queue was read: the proposals present then are not news. */
  loaded: false,
  /** Counts the reads of the queue that came through, so a watcher can tell a fresh read from the list it had. */
  revision: 0,
  /** Why the queue could not be read, if it could not. */
  error: null as string | null,
  /** All proposals, newest first. Kept current only while a page watches it (watchHistory). */
  history: [] as ApprovalRecord[],
  historyLoaded: false,
  historyError: null as string | null,
})

let queueSeq = 0

/** Reloads the waiting proposals. Only the newest call writes, so an older answer never undoes a newer one. */
export async function refreshApprovals(): Promise<void> {
  const seq = ++queueSeq
  try {
    const list = inApp() ? await invoke<Proposal[]>('approvals_list') : []
    if (seq !== queueSeq) return
    store.approvals = Array.isArray(list) ? list : []
    approvalState.error = null
    approvalState.loaded = true
    approvalState.revision += 1
  } catch (error) {
    if (seq !== queueSeq) return
    approvalState.error = String(error)
  }
}

let historySeq = 0
let historyWatchers = 0

/** Reloads the history, newest first. */
export async function refreshHistory(): Promise<void> {
  const seq = ++historySeq
  try {
    const list = inApp() ? await invoke<ApprovalRecord[]>('approvals_history', {}) : []
    if (seq !== historySeq) return
    approvalState.history = Array.isArray(list) ? list : []
    approvalState.historyError = null
    approvalState.historyLoaded = true
  } catch (error) {
    if (seq !== historySeq) return
    approvalState.historyError = String(error)
    approvalState.historyLoaded = true
  }
}

/** Loads the history now and keeps it current on every change, until the returned function is called. */
export function watchHistory(): () => void {
  historyWatchers += 1
  void refreshHistory()
  let stopped = false
  return () => {
    if (stopped) return
    stopped = true
    historyWatchers -= 1
  }
}

let listening = false

/** Loads the queue and follows it (and, while watched, the history) on the app's `approvalsChanged` events. */
export async function connectApprovals(): Promise<void> {
  if (!listening && inApp()) {
    listening = true
    await listen<{ type: string }>('state', (event) => {
      if (event.payload?.type !== 'approvalsChanged') return
      void refreshApprovals()
      if (historyWatchers > 0) void refreshHistory()
    })
  }
  await refreshApprovals()
}

/** The user's answer to one waiting change. Only `approve` lets it go to e-Boekhouden. Rejects when it is gone. */
export const decideApproval = (id: number, approve: boolean): Promise<void> =>
  invoke<void>('approval_decide', { id, approve }).finally(refreshApprovals)

/** Answers exactly these proposals, and no others that came in meanwhile. */
export const decideAll = (ids: number[], approve: boolean): Promise<DecideAllResult> =>
  invoke<DecideAllResult>('approvals_decide_all', { ids, approve }).finally(refreshApprovals)

export const approvalDetail = (id: number): Promise<ApprovalDetail> => invoke<ApprovalDetail>('approval_get', { id })

/** Writes the history to a .jsonl.gz file and gives its absolute path. */
export const exportApprovals = (): Promise<string> => invoke<string>('approvals_export')

export type StatusTone = 'ok' | 'bad' | 'run' | 'off'

export const STATUSES: { value: ApprovalStatus; label: string; tone: StatusTone }[] = [
  { value: 'waiting', label: 'Wacht', tone: 'run' },
  { value: 'running', label: 'Bezig', tone: 'run' },
  { value: 'done', label: 'Uitgevoerd', tone: 'ok' },
  { value: 'failed', label: 'Mislukt', tone: 'bad' },
  { value: 'refused', label: 'Geweigerd', tone: 'off' },
  { value: 'lapsed', label: 'Vervallen', tone: 'off' },
]

export function statusOf(status: ApprovalStatus): { label: string; tone: StatusTone } {
  return STATUSES.find((s) => s.value === status) ?? { label: status, tone: 'off' }
}

/** How the detail names the answer, and whether it was given to all waiting changes at once. */
export function decisionLabel(decision: ApprovalDecision | null, bulk = false): string {
  switch (decision) {
    case 'approved': return bulk ? 'Goedgekeurd · Alles goedkeuren' : 'Goedgekeurd'
    case 'refused': return bulk ? 'Geweigerd · Alles weigeren' : 'Geweigerd'
    case 'auto': return 'Zonder akkoord: dit account boekt direct'
    case 'lapsed': return 'Niet beantwoord'
    default: return 'Nog niet beantwoord'
  }
}

/** Records with this status (all when empty) whose title, account or proposal number holds every word of the query. */
export function filterHistory(records: ApprovalRecord[], status: ApprovalStatus | '', query: string): ApprovalRecord[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  return records.filter((record) => {
    if (status && record.status !== status) return false
    const text = `${record.title} ${record.account} ${record.id}`.toLowerCase()
    return words.every((word) => text.includes(word))
  })
}

/** How a proposal's tint reads in the queue: a new thing, a plain change, or one that cannot be undone. */
export type KindTone = 'accent' | 'neutral' | 'danger'

/** The icon and tint the queue shows for a proposal, from its tool name. */
export interface ProposalKind {
  icon: string
  tone: KindTone
}

/**
 * What kind of change a proposal makes, for the icon in the queue: adding or uploading gets a plus on the accent tint,
 * changing or linking a pencil, removing a trash can, mailing an invoice an envelope, crediting or invoicing a
 * document, anything else a generic mark. A change that cannot be undone is always on the red tint.
 */
export function proposalKind(proposal: Pick<Proposal, 'tool' | 'danger'>): ProposalKind {
  const kind = kindOfTool(proposal.tool)
  return proposal.danger ? { icon: kind.icon, tone: 'danger' } : kind
}

function kindOfTool(tool: string): ProposalKind {
  // By what the name ends in, so a tool added later (offerte_mailen, archief_ontkoppelen) gets its icon by itself.
  if (tool.endsWith('_mailen') || tool.endsWith('_versturen')) return { icon: 'mail', tone: 'neutral' }
  if (tool.endsWith('_crediteren') || tool.endsWith('_factureren')) return { icon: 'file-text', tone: 'neutral' }
  if (tool.endsWith('_toevoegen') || tool.endsWith('_uploaden')) return { icon: 'plus', tone: 'accent' }
  if (tool.endsWith('_wijzigen') || tool.endsWith('_koppelen') || tool.endsWith('_ontkoppelen')) return { icon: 'pencil', tone: 'neutral' }
  if (tool.endsWith('_verwijderen')) return { icon: 'trash', tone: 'neutral' }
  return { icon: 'square-dot', tone: 'neutral' }
}

/**
 * A title as "what happens: subject" in two parts, split on the first ": ", as most tools write it
 * ("Relatie verwijderen: EB MCP proef wachtrij"). Without one, or with an empty side, the whole title is what happens.
 */
export function splitTitle(title: string): { what: string; subject: string | null } {
  const at = title.indexOf(': ')
  if (at < 0) return { what: title, subject: null }
  const what = title.slice(0, at).trim()
  const subject = title.slice(at + 2).trim()
  return what && subject ? { what, subject } : { what: title, subject: null }
}

/**
 * Which proposal is selected after the queue changed from `before` to `next` (ids in list order). The selected one
 * stays when it still waits. When it left, the one that now takes its place is selected, or the one before it when it
 * was the last; when nothing was selected, or it was not in the list, the first.
 */
export function followSelection(before: number[], next: number[], current: number | null): number | null {
  if (!next.length) return null
  if (current !== null && next.includes(current)) return current
  const at = current === null ? -1 : before.indexOf(current)
  if (at < 0) return next[0]!
  const still = new Set(next)
  // The ones above it that still wait keep their places, so its successor sits right after them.
  const above = before.slice(0, at).filter((id) => still.has(id)).length
  return next[Math.min(above, next.length - 1)]!
}

/** "1 wijziging wacht op je akkoord", "3 wijzigingen wachten op je akkoord". */
export function waitingText(n: number): string {
  return n === 1 ? '1 wijziging wacht op je akkoord' : `${n} wijzigingen wachten op je akkoord`
}

/** What stopping EB MCP now costs: calls that still run fail, and changes that wait for approval lapse. */
export function quitWarning(running: number, waiting: number): string {
  const parts: string[] = []
  if (running) parts.push(`${running === 1 ? 'Er loopt nog 1 aanroep' : `Er lopen nog ${running} aanroepen`} van een AI-client; die ${running === 1 ? 'krijgt' : 'krijgen'} dan een fout.`)
  if (waiting) parts.push(`${waitingText(waiting)}; die ${waiting === 1 ? 'vervalt' : 'vervallen'} dan en ${waiting === 1 ? 'wordt' : 'worden'} niet uitgevoerd.`)
  parts.push('EB MCP vergeet alle sessies.')
  return parts.join(' ')
}

/** "zojuist", "12 min geleden", "2 u 30 min geleden". */
export function ago(ms: number, now = Date.now()): string {
  const text = since(ms, now)
  return text === 'net' ? 'zojuist' : `${text} geleden`
}

/** "Studio Noord", "Studio Noord en Bakkerij Zuid", "A, B en C". */
export function nameList(names: string[]): string {
  const unique = [...new Set(names)]
  if (unique.length <= 1) return unique[0] ?? ''
  return `${unique.slice(0, -1).join(', ')} en ${unique.at(-1)}`
}

/** "11 nov. 14:32" in lists; the full date and time in the detail. */
export function stamp(ms: number, full = false): string {
  return full
    ? new Date(ms).toLocaleString('nl-NL', { dateStyle: 'long', timeStyle: 'medium' })
    : new Date(ms).toLocaleString('nl-NL', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}
