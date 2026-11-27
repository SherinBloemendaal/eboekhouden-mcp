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
  /** The file the change sends along (an upload to the digitaal archief), shown before Goedkeuren. */
  file?: Attachment
  /** The statement the change reads in (afschrift_inlezen): checked line by line on its own sheet before Goedkeuren. */
  statement?: StatementInfo
}

/** What a proposal says of its statement without the lines (statement.rs, `Info`). */
export interface StatementInfo {
  fileName: string
  lines: number
  included: number
}

/** A file from this computer that a change sends to e-Boekhouden (approvals.rs, `Attachment`). */
export interface Attachment {
  path: string
  name: string
  size: number
  /** 'application/pdf' or an image type when the preview can show it, told from the file's bytes; null: no preview. */
  kind: string | null
  /** Openen may hand the file to its own app: only a PDF or an image. Anything else is only shown in its folder. */
  openable: boolean
}

/** The file as the proposal read it (`approval_file`), which is what goes out; `changed`: the file on disk is no longer that, or is gone. */
export interface FilePreview {
  name: string
  kind: string | null
  size: number
  changed: boolean
  /** Base64. */
  data: string
}

export type ApprovalStatus = 'waiting' | 'running' | 'done' | 'failed' | 'refused' | 'lapsed' | 'withdrawn' | 'returned'
/** How the proposal was answered: by the user, without approval (the account books freely), or not in time. */
export type ApprovalDecision = 'approved' | 'refused' | 'auto' | 'lapsed' | 'withdrawn' | 'returned'

/** A proposal in the history, waiting or answered, with what came of it. */
export interface ApprovalRecord extends Proposal {
  decidedAt: number | null
  decision: ApprovalDecision | null
  status: ApprovalStatus
  error: string | null
  /** What did not come of a change that is done: the change stands, a part that goes with it (an attachment) does not. */
  note?: string | null
  /** What the user wrote when sending the proposal back to the AI client. */
  remark?: string | null
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

/** Sends a waiting proposal back to the AI client with a remark: nothing goes to e-Boekhouden. Rejects with a Dutch sentence. */
export const returnApproval = (id: number, remark: string): Promise<void> =>
  invoke<void>('approval_return', { id, remark }).finally(refreshApprovals)

/** Answers exactly these proposals, and no others that came in meanwhile. */
export const decideAll = (ids: number[], approve: boolean): Promise<DecideAllResult> =>
  invoke<DecideAllResult>('approvals_decide_all', { ids, approve }).finally(refreshApprovals)

export const approvalDetail = (id: number): Promise<ApprovalDetail> => invoke<ApprovalDetail>('approval_get', { id })

/** Writes the history to a .jsonl.gz file and gives its absolute path. */
export const exportApprovals = (): Promise<string> => invoke<string>('approvals_export')

export type StatusTone = 'ok' | 'bad' | 'warn' | 'run' | 'off' | 'back'

export const STATUSES: { value: ApprovalStatus; label: string; tone: StatusTone }[] = [
  { value: 'waiting', label: 'Wacht', tone: 'run' },
  { value: 'running', label: 'Bezig', tone: 'run' },
  { value: 'done', label: 'Uitgevoerd', tone: 'ok' },
  { value: 'failed', label: 'Mislukt', tone: 'bad' },
  { value: 'refused', label: 'Geweigerd', tone: 'off' },
  { value: 'lapsed', label: 'Vervallen', tone: 'off' },
  // Taken back by the AI client itself (akkoord_intrekken) before the user answered.
  { value: 'withdrawn', label: 'Ingetrokken', tone: 'off' },
  // Sent back to the AI client by the user, with a remark, instead of approved or refused.
  { value: 'returned', label: 'Teruggestuurd', tone: 'back' },
]

export function statusOf(status: ApprovalStatus): { label: string; tone: StatusTone } {
  return STATUSES.find((s) => s.value === status) ?? { label: status, tone: 'off' }
}

/** The status of a record as its chip shows it: a change that is done but left a part undone says so. */
export function recordStatus(record: Pick<ApprovalRecord, 'status' | 'note'>): { label: string; tone: StatusTone } {
  return record.status === 'done' && record.note ? { label: 'Deels uitgevoerd', tone: 'warn' } : statusOf(record.status)
}

/** How the detail names the answer, and whether it was given to all waiting changes at once. */
export function decisionLabel(decision: ApprovalDecision | null, bulk = false): string {
  switch (decision) {
    case 'approved': return bulk ? 'Goedgekeurd · Alles goedkeuren' : 'Goedgekeurd'
    case 'refused': return bulk ? 'Geweigerd · Alles weigeren' : 'Geweigerd'
    case 'auto': return 'Zonder akkoord: dit account boekt direct'
    case 'lapsed': return 'Niet beantwoord'
    case 'withdrawn': return 'Ingetrokken door de AI-client'
    case 'returned': return 'Teruggestuurd naar de AI-client'
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
  if (tool.endsWith('_inlezen')) return { icon: 'list-checks', tone: 'accent' }
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

/** The file of a proposal that still waits, for its preview. Rejects with a Dutch sentence. */
export const approvalFile = (id: number): Promise<FilePreview> => invoke<FilePreview>('approval_file', { id })
/** Opens the file of a waiting proposal in its own app, or shows it in Finder or Explorer with `reveal`. */
export const openApprovalFile = (id: number, reveal = false): Promise<void> => invoke<void>('approval_file_open', { id, reveal })

/** What the preview can show of a file: an image, a PDF, or nothing. */
export function previewKind(kind: string | null | undefined): 'image' | 'pdf' | null {
  if (kind === 'application/pdf') return 'pdf'
  return kind?.startsWith('image/') ? 'image' : null
}

// ---------------------------------------------------------------- the statement of a proposal (statement.rs)

/** Year, month, day. */
export type Ymd = [number, number, number]

/** What a rule that leaves lines out looks at (statement.rs, `Test`). */
export type StatementTest =
  | { kind: 'contains'; text: string }
  | { kind: 'column'; column: number; value: string }
  | { kind: 'until'; date: Ymd }
  | { kind: 'from'; date: Ymd }
  | { kind: 'amount'; cents: number }
  | { kind: 'incoming' }
  | { kind: 'outgoing' }
  | { kind: 'cancelling' }

export type StatementRule = StatementTest & {
  on: boolean
  /** Who put it there: the AI client as a proposal, or the user. */
  by: 'client' | 'user'
  why?: string
  label: string
  /** The lines it leaves out now. */
  count: number
}

export type StatementOut =
  | { kind: 'beforeStart' }
  | { kind: 'unreadable'; what: string }
  | { kind: 'rule'; rule: number }
  | { kind: 'user' }

export interface StatementRow {
  line: number
  cells: string[]
  /** ISO, or null when the chosen column holds no date. */
  date: string | null
  description: string
  cents: number | null
  included: boolean
  out: StatementOut | null
  /** The rule that would leave it out, when the user ticked it all the same. */
  overruled: number | null
}

export interface StatementMapping {
  date: number
  description: number
  amount: number
  /** The first line that counts. */
  start: number
}

export interface StatementView {
  fileName: string
  header: string[]
  delimiter: string
  empty: number
  mapping: StatementMapping
  rules: StatementRule[]
  dedupe: boolean
  ledgers: { id: number; code: string; name: string }[]
  ledger: number
  rows: StatementRow[]
  totals: {
    lines: number
    included: number
    skipped: number
    unreadable: number
    cents: number
    first: string | null
    last: string | null
    perRule: number[]
  }
}

export type StatementEdit =
  | { kind: 'mapping'; mapping: StatementMapping }
  | { kind: 'ledger'; index: number }
  | { kind: 'dedupe'; on: boolean }
  | { kind: 'addRule'; test: StatementTest }
  | { kind: 'toggleRule'; index: number; on: boolean }
  | { kind: 'removeRule'; index: number }
  | { kind: 'tick'; lines: number[]; included: boolean }
  | { kind: 'clearTicks' }

/** The statement of a proposal that still waits. Rejects with a Dutch sentence. */
export const approvalStatement = (id: number): Promise<StatementView> => invoke<StatementView>('approval_statement', { id })
/** One change on the sheet; gives the statement as it is after it. */
export const editStatement = (id: number, edit: StatementEdit): Promise<StatementView> =>
  invoke<StatementView>('approval_statement_edit', { id, edit })

/** Cents as "1.234,56" or "-24,00". */
export function euroCents(cents: number): string {
  const abs = Math.abs(cents)
  const whole = Math.floor(abs / 100).toString().replace(/\B(?=(\d{3})+(?!\d))/g, '.')
  return `${cents < 0 ? '-' : ''}${whole},${String(abs % 100).padStart(2, '0')}`
}

/** "2026-11-06" as "06-11-2026". */
export function dutchDate(iso: string | null): string {
  if (!iso) return ''
  const [y, m, d] = iso.split('-')
  return `${d}-${m}-${y}`
}

/** Why a line does not go along, in a few words. */
export function outReason(out: StatementOut | null, rules: StatementRule[]): string {
  switch (out?.kind) {
    case 'beforeStart': return 'Voor de startregel'
    case 'unreadable': return `Niet te lezen: ${out.what}`
    case 'rule': return rules[out.rule]?.label ?? 'Regel'
    case 'user': return 'Zelf uitgezet'
    default: return ''
  }
}

/** The rows the sheet lists: by tab, and by the words typed in the search field. Lines before the start line are left out of every tab. */
export function filterRows(rows: StatementRow[], tab: 'all' | 'in' | 'out', query: string): StatementRow[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  return rows.filter((row) => {
    if (row.out?.kind === 'beforeStart') return false
    if (tab === 'in' && !row.included) return false
    if (tab === 'out' && row.included) return false
    const text = row.cells.join(' ').toLowerCase()
    return words.every((word) => text.includes(word))
  })
}

/** Whether a line would be left out by this test: what the sheet highlights while a rule is being made. */
export function testHits(test: StatementTest, row: StatementRow): boolean {
  if (row.out?.kind === 'beforeStart' || row.out?.kind === 'unreadable') return false
  switch (test.kind) {
    case 'contains': return test.text.trim() !== '' && row.description.toLowerCase().includes(test.text.trim().toLowerCase())
    case 'column': return test.value.trim() !== '' && (row.cells[test.column] ?? '').trim().toLowerCase() === test.value.trim().toLowerCase()
    case 'until': return row.date !== null && row.date <= isoOf(test.date)
    case 'from': return row.date !== null && row.date >= isoOf(test.date)
    case 'amount': return row.cents === test.cents
    case 'incoming': return (row.cents ?? 0) > 0
    case 'outgoing': return (row.cents ?? 0) < 0
    // Pairs are worked out by EB MCP itself; nothing to highlight before the rule is there.
    case 'cancelling': return false
  }
}

const isoOf = ([y, m, d]: Ymd) => `${y}-${String(m).padStart(2, '0')}-${String(d).padStart(2, '0')}`
