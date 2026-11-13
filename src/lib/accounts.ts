import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { reactive } from 'vue'
import type { Proposal } from './approvals'
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
  /** What AI clients may do in this account's books besides reading: a level per kind of change. */
  rights: Rights
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
  rights: Rights
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
  | 'queued' | 'credentials' | 'submit' | 'code' | 'administration' | 'done' | 'failed' | 'unknown' | 'cancelled'

export interface SigninEvent {
  accountId: string
  step: Step
  message: string | null
}

export type Appearance = 'system' | 'light' | 'dark'

/** How far AI clients may go with one kind of change: refused, each one with approval, or at once. */
export type Level = 'off' | 'ask' | 'auto'

/** The kinds of change an account sets a level for (accounts.rs). */
export type Kind = 'change' | 'delete' | 'send' | 'heavy'

export type Rights = Record<Kind, Level>

export const LEVEL_LABELS: Record<Level, string> = { off: 'Uit', ask: 'Met akkoord', auto: 'Zonder akkoord' }

/** The kinds as the app names them, with the levels each can be set to: what leaves or reaches far never goes without approval. */
export const KINDS: { kind: Kind; label: string; hint: string; levels: Level[] }[] = [
  { kind: 'change', label: 'Boeken en wijzigen', hint: 'Mutaties, facturen, relaties en al het andere toevoegen of wijzigen. Is terug te draaien.', levels: ['off', 'ask', 'auto'] },
  { kind: 'delete', label: 'Verwijderen', hint: 'Wat weg is, komt niet terug.', levels: ['off', 'ask', 'auto'] },
  { kind: 'send', label: 'Versturen', hint: 'Mail, Peppol, het incassobestand en berichten aan de accountant. Gaat nooit zonder akkoord.', levels: ['off', 'ask'] },
  { kind: 'heavy', label: 'Ingrijpend', hint: 'Crediteren, in bulk factureren of boeken, afboeken, de beginbalans, bedrijfsgegevens en een btw-aangifte verwerken. Gaat nooit zonder akkoord.', levels: ['off', 'ask'] },
]

const all = (level: Level): Rights => ({ change: level, delete: level, send: level, heavy: level })

/** The usual choices in one click; the rows under them say exactly what each sets. */
export const PRESETS: { id: string; label: string; hint: string; rights: Rights }[] = [
  { id: 'off', label: 'Alleen lezen', hint: 'AI-clients kunnen in dit account niets wijzigen.', rights: all('off') },
  { id: 'ask', label: 'Alles met akkoord', hint: 'Elke wijziging komt eerst bij jou in EB MCP; pas na Goedkeuren gaat hij naar e-Boekhouden.', rights: all('ask') },
  { id: 'auto', label: 'Vrij boeken', hint: 'AI-clients boeken en wijzigen direct. Verwijderen, versturen en wat ingrijpend is vragen nog wel je akkoord.', rights: { ...all('ask'), change: 'auto' } },
]

/** The preset these rights are, or `null` when the user set the kinds by hand. */
export function presetOf(rights: Rights): (typeof PRESETS)[number] | null {
  return PRESETS.find((p) => KINDS.every((k) => p.rights[k.kind] === rights[k.kind])) ?? null
}

/** Whether AI clients can change nothing at all. */
export function readOnly(rights: Rights): boolean {
  return KINDS.every((k) => rights[k.kind] === 'off')
}

/** The rights in a few words for the chip on an account, `null` for read-only. Red when something goes without approval. */
export function rightsChip(rights: Rights): { label: string; tone: 'info' | 'warn' } | null {
  if (readOnly(rights)) return null
  const free = KINDS.filter((k) => rights[k.kind] === 'auto')
  if (free.some((k) => k.kind === 'delete')) return { label: free.length > 1 ? 'Boekt en verwijdert zelf' : 'Verwijdert zelf', tone: 'warn' }
  if (free.length) return { label: 'Boekt zelf', tone: 'warn' }
  if (rights.change === 'off') return { label: 'Beperkt met akkoord', tone: 'info' }
  return { label: 'Boekt met akkoord', tone: 'info' }
}

/** Every kind with its level, for a tooltip: "Boeken en wijzigen: met akkoord · Verwijderen: uit · ...". */
export function rightsText(rights: Rights): string {
  return KINDS.map((k) => `${k.label}: ${LEVEL_LABELS[rights[k.kind]].toLowerCase()}`).join(' · ')
}

export interface Settings {
  idleMinutes: number
  /** Blijf ingelogd: no idle sign-out, and EB MCP keeps each session alive at e-Boekhouden. */
  keepSignedIn: boolean
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
  /** Changes that wait for the user's approval, oldest first: the one list every page and sign reads (approvals.ts). */
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

/** Loads the accounts and starts listening once per window; later calls only refresh. */
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
export const signIn = (id: string) => {
  // Shown at once, in place of an earlier sign-in's end state; the app's own first step follows.
  store.signins[id] = { accountId: id, step: 'credentials', message: null }
  markStarted(id)
  return invoke('signin_start', { id })
}
export const cancelSignIn = (id: string) => invoke('signin_cancel', { id })

/**
 * Alles inloggen: starts a sign-in for every account without a session that stands. The app runs a few at a
 * time; the others wait for their turn. Gives the accounts it started.
 */
export async function signInAll(): Promise<string[]> {
  const ids = await invoke<string[]>('signin_all')
  for (const id of ids) {
    // Shown at once; the app's own steps follow. An account whose sign-in already ran keeps its newest step.
    if (!isRunning(store.signins[id]?.step)) store.signins[id] = { accountId: id, step: 'queued', message: null }
    markStarted(id)
  }
  return ids
}

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
export const getSettings = () => (inApp() ? invoke<Settings>('settings_get') : Promise.resolve<Settings>({ idleMinutes: 30, keepSignedIn: false, appearance: 'system', showInDock: true }))
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
