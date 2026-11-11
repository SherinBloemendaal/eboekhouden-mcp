import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { reactive } from 'vue'
import { inApp } from './server'

export type StoreKind = 'onePassword' | 'system'

export interface SessionView {
  accountId: string
  label: string
  administration: string | null
  startedAt: number
  expired: boolean
}

export interface AccountView {
  id: string
  label: string
  username: string
  store: StoreKind
  opRef: string | null
  opTitle: string | null
  opVault: string | null
  administration: string | null
  /** Whether AI clients may change this account's books: not at all, each change with approval, or freely. */
  write: WriteMode
  session: SessionView | null
  signingIn: boolean
  /** The newest step of this account's latest sign-in since EB MCP started, its end state included. */
  signin: SigninEvent | null
}

export interface AccountInput {
  label: string
  username: string
  store: StoreKind
  opRef: string | null
  opTitle: string | null
  opVault: string | null
  write: WriteMode
}

/** A Login item in 1Password, as `op item list` gives it. Never carries a password. */
export interface OpItem {
  /** `op://<vault id>/<item id>`: keeps working when the item is renamed. */
  reference: string
  title: string
  vault: string
  username: string
  /** For e-Boekhouden: its website or title says so. */
  suggested: boolean
}

export type OpListError = { kind: 'missing' } | { kind: 'locked' } | { kind: 'other'; message: string }

export const opItems = () => invoke<OpItem[]>('op_items')
export const openOpInstall = () => (inApp() ? invoke('open_op_install') : Promise.resolve())

/** Every word of the query must appear in the title, username or vault; e-Boekhouden items come first. */
export function filterOpItems(items: OpItem[], query: string): { suggested: OpItem[]; other: OpItem[] } {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  const hits = items.filter((item) => {
    const text = `${item.title} ${item.username} ${item.vault}`.toLowerCase()
    return words.every((word) => text.includes(word))
  })
  return { suggested: hits.filter((i) => i.suggested), other: hits.filter((i) => !i.suggested) }
}

/** How the list names a 1Password item: "Kluis · Titel" when it was picked, else the reference. */
export function opLabel(account: Pick<AccountView, 'opRef' | 'opTitle' | 'opVault'>): string {
  if (account.opTitle) return account.opVault ? `${account.opVault} · ${account.opTitle}` : account.opTitle
  return account.opRef ?? ''
}

export type Step =
  | 'credentials' | 'submit' | 'code' | 'administration' | 'done' | 'failed' | 'unknown' | 'cancelled'

export interface SigninEvent {
  accountId: string
  step: Step
  message: string | null
}

export type Appearance = 'system' | 'light' | 'dark'

export type WriteMode = 'off' | 'ask' | 'auto'

/** How the app names an account's write setting. */
export const WRITE_MODES: { value: WriteMode; label: string; hint: string }[] = [
  { value: 'off', label: 'Alleen lezen', hint: 'AI-clients kunnen in dit account niets wijzigen.' },
  { value: 'ask', label: 'Boeken met akkoord', hint: 'Elke wijziging komt eerst bij jou in EB MCP; pas na Goedkeuren gaat hij naar e-Boekhouden.' },
  { value: 'auto', label: 'Boeken zonder akkoord', hint: 'AI-clients boeken en wijzigen direct. Verwijderen, crediteren en abonnementen factureren vragen nog wel je akkoord.' },
]

/** A change an AI client asked for that waits for the user's approval (approvals.rs). */
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

export interface Settings {
  idleMinutes: number
  appearance: Appearance
  /** macOS: an icon in the Dock as well as in the menu bar. */
  showInDock: boolean
}

/** One shared, reactive view of the accounts and their sign-ins. */
export const store = reactive({
  accounts: [] as AccountView[],
  loaded: false,
  /** The latest step per account while signing in, and the end state until the user closes it. */
  signins: {} as Record<string, SigninEvent>,
  /** Changes that wait for the user's approval, oldest first. */
  approvals: [] as Proposal[],
})

let refreshSeq = 0

/**
 * Reloads the accounts and, with them, the newest step of each account's latest sign-in. The app keeps that step,
 * its end state included, so a window that missed an event (it opened later, or was asleep) still shows what
 * happened. Only the newest call writes: a list read before a sign-in event arrived would otherwise undo what that
 * event showed.
 */
export async function refresh(): Promise<void> {
  const seq = ++refreshSeq
  const accounts = inApp() ? await invoke<AccountView[]>('accounts_list') : []
  if (seq !== refreshSeq) return
  store.accounts = accounts
  store.loaded = true
  for (const account of accounts) {
    if (account.signin) take(account.signin)
    else {
      delete store.signins[account.id]
      codeAsked.delete(account.id)
    }
  }
}

/** Stores an account's newest sign-in step and keeps track of a running sign-in that asked for a code. */
function take(event: SigninEvent): void {
  if (event.step === 'code') codeAsked.add(event.accountId)
  else if (!isRunning(event.step)) codeAsked.delete(event.accountId)
  store.signins[event.accountId] = event
}

/** Whether a sign-in of this account runs: by its newest step when this window knows one, else as the app said. */
export function signingIn(account: AccountView): boolean {
  const event = store.signins[account.id]
  return event ? isRunning(event.step) : account.signingIn
}

/** Sign-ins this window started (or added the account for): it shows their steps, until Sluiten. */
const startedSet = reactive(new Set<string>())
/** Running sign-ins that asked this window for a code: it shows their steps wherever they started. */
const codeAsked = reactive(new Set<string>())

/** Marks a sign-in of this account as started in this window. */
export function markStarted(id: string): void {
  startedSet.add(id)
}

export function startedHere(id: string): boolean {
  return startedSet.has(id)
}

/** Whether the running sign-in of this account asked for a code. */
export function askedCode(id: string): boolean {
  return codeAsked.has(id)
}

/**
 * Takes in one `signin` event. Every window gets every event and keeps the newest one per account, the end state
 * too: which window shows the steps is each window's own choice (the one that started or follows the sign-in),
 * but none may be left showing a step that is no longer the newest.
 */
export function receiveSignin(event: SigninEvent): void {
  take(event)
}

let listening = false

/** Starts listening once per window; later calls only refresh. */
/** Reloads the changes that wait for approval. Apart from `refresh`: they change on their own events. */
export async function refreshApprovals(): Promise<void> {
  const list = inApp() ? await invoke<Proposal[]>('approvals_list') : []
  store.approvals = Array.isArray(list) ? list : []
}

/** The user's answer to a waiting change. Only `approve` lets it go to e-Boekhouden. */
export const decideApproval = (id: number, approve: boolean) =>
  invoke('approval_decide', { id, approve }).finally(refreshApprovals)

export async function connect(): Promise<void> {
  await refresh()
  await refreshApprovals()
  if (listening || !inApp()) return
  listening = true
  await listen<{ type: string }>('state', (event) => {
    void refresh()
    if (event.payload?.type === 'approvalsChanged') void refreshApprovals()
  })
  await listen<SigninEvent>('signin', (event) => {
    receiveSignin(event.payload)
    void refresh()
  })
}

// A new account signs in once to check its credentials; this window asked for that sign-in.
export const addAccount = (input: AccountInput, password: string | null) =>
  invoke<AccountView>('account_add', { input, password })
    .then((account) => {
      markStarted(account.id)
      return account
    })
    .finally(refresh)
export const updateAccount = (id: string, input: AccountInput, password: string | null) =>
  invoke<AccountView>('account_update', { id, input, password }).finally(refresh)
export const removeAccount = (id: string) => invoke('account_remove', { id }).finally(refresh)
export const signIn = (id: string) => {
  // Shown at once, in place of an earlier sign-in's end state; the app's own first step follows.
  store.signins[id] = { accountId: id, step: 'credentials', message: null }
  markStarted(id)
  return invoke('signin_start', { id })
}
export const cancelSignIn = (id: string) => invoke('signin_cancel', { id })

/** One sign-in as Instellingen › Diagnose shows it: what EB MCP saw and did. No password, code or cookie. */
export interface Trace {
  accountId: string
  label: string
  startedAt: number
  outcome: string | null
  lines: { ms: number; text: string }[]
}
export const signinTraces = () => (inApp() ? invoke<Trace[]>('signin_traces') : Promise.resolve<Trace[]>([]))

/** A trace as plain text, to paste into a message. */
export function traceText(trace: Trace): string {
  const head = `${trace.label}, ${new Date(trace.startedAt).toLocaleString('nl-NL')}: ${trace.outcome ?? 'loopt nog'}`
  return [head, ...trace.lines.map((l) => `${(l.ms / 1000).toFixed(1).padStart(6)} s  ${l.text}`)].join('\n')
}
export const sendCode = (id: string, code: string) => invoke('signin_code', { id, code })
export const signOut = (id: string) => invoke('sign_out', { id }).finally(refresh)
export const signOutAll = () => invoke('sign_out_all').finally(refresh)
export const getSettings = () => (inApp() ? invoke<Settings>('settings_get') : Promise.resolve<Settings>({ idleMinutes: 30, appearance: 'system', showInDock: true }))
export const saveSettings = (value: Settings) => invoke('settings_set', { value })

/** Closes this window's view of a sign-in. Its newest step stays known, here and in other windows. */
export function dismissSignin(id: string): void {
  startedSet.delete(id)
  codeAsked.delete(id)
}

/** Initials for the avatar: first letters of the first two words. */
export function initials(label: string): string {
  const words = label.trim().split(/\s+/).filter(Boolean)
  return ((words[0]?.[0] ?? '') + (words[1]?.[0] ?? words[0]?.[1] ?? '')).toUpperCase()
}

/** "2 u 30 min", "2 u", "12 min", "net". */
export function since(ms: number, now = Date.now()): string {
  const minutes = Math.floor((now - ms) / 60000)
  if (minutes < 1) return 'net'
  const h = Math.floor(minutes / 60)
  const m = minutes % 60
  if (!h) return `${m} min`
  return m ? `${h} u ${m} min` : `${h} u`
}

/** "Zojuist ingelogd", "12 min ingelogd", "2 u ingelogd", "2 u 30 min ingelogd". */
export function sessionAge(startedAt: number, now = Date.now()): string {
  return now - startedAt < 60_000 ? 'Zojuist ingelogd' : `${since(startedAt, now)} ingelogd`
}

export type OutcomeTone = 'ok' | 'bad' | 'run' | 'off'

/**
 * A trace's outcome in words: "Done", "Done: Naam", "Failed: ...", "Unknown: ...", "Cancelled", or null while it
 * runs. Anything else did not end well either, so it reads as an unknown error.
 */
export function outcomeLabel(outcome: string | null): { text: string; tone: OutcomeTone } {
  if (outcome === null) return { text: 'Loopt', tone: 'run' }
  const kind = outcome.split(':', 1)[0]!.trim()
  if (kind === 'Done') return { text: 'Gelukt', tone: 'ok' }
  if (kind === 'Failed') return { text: 'Mislukt', tone: 'bad' }
  if (kind === 'Cancelled') return { text: 'Geannuleerd', tone: 'off' }
  return { text: 'Onbekende fout', tone: 'bad' }
}

export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString('nl-NL', { hour: '2-digit', minute: '2-digit' })
}

export function isRunning(step: Step | undefined): boolean {
  return step !== undefined && !['done', 'failed', 'unknown', 'cancelled'].includes(step)
}
