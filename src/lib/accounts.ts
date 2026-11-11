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
  isDefault: boolean
  session: SessionView | null
  signingIn: boolean
}

export interface AccountInput {
  label: string
  username: string
  store: StoreKind
  opRef: string | null
  opTitle: string | null
  opVault: string | null
  isDefault: boolean
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
})

let refreshSeq = 0

/**
 * Reloads the accounts. Only the newest call writes: a list read before a sign-in event arrived would otherwise
 * undo what that event showed.
 */
export async function refresh(): Promise<void> {
  const seq = ++refreshSeq
  const accounts = inApp() ? await invoke<AccountView[]>('accounts_list') : []
  if (seq !== refreshSeq) return
  store.accounts = accounts
  store.loaded = true
  for (const account of accounts) {
    const event = store.signins[account.id]
    if (account.signingIn && !event) {
      // This window opened while a sign-in ran: show it as running, so the row offers Annuleren and not an idle Inloggen.
      store.signins[account.id] = { accountId: account.id, step: 'submit', message: null }
    } else if (!account.signingIn && event && isRunning(event.step)) {
      // The sign-in stopped and its last step never reached this window.
      delete store.signins[account.id]
      codeAsked.delete(account.id)
    }
  }
}

/** Sign-ins this window started (or added the account for). Only these keep their end state until Sluiten. */
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
 * Takes in one `signin` event. Every window gets every event: running steps are kept for every account, so each
 * window shows the account as busy. An end state is kept only where the sign-in started; elsewhere it is dropped,
 * so no finished steps linger in a window that never asked for them.
 */
export function receiveSignin(event: SigninEvent): void {
  const id = event.accountId
  if (isRunning(event.step)) {
    if (event.step === 'code') codeAsked.add(id)
    store.signins[id] = event
    return
  }
  codeAsked.delete(id)
  if (startedSet.delete(id)) store.signins[id] = event
  else delete store.signins[id]
}

let listening = false

/** Starts listening once per window; later calls only refresh. */
export async function connect(): Promise<void> {
  await refresh()
  if (listening || !inApp()) return
  listening = true
  await listen('state', () => void refresh())
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
export const setDefault = (id: string) => invoke('account_set_default', { id }).finally(refresh)
export const signIn = (id: string) => {
  delete store.signins[id]
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

export function dismissSignin(id: string): void {
  delete store.signins[id]
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
