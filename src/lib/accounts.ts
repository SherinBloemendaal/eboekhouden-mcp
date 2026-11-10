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
  renewedAt: number | null
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
  | 'credentials' | 'page' | 'submit' | 'code' | 'administration' | 'done' | 'failed' | 'unknown' | 'cancelled'

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

export async function refresh(): Promise<void> {
  store.accounts = inApp() ? await invoke<AccountView[]>('accounts_list') : []
  store.loaded = true
}

let listening = false

/** Starts listening once per window; later calls only refresh. */
export async function connect(): Promise<void> {
  await refresh()
  if (listening || !inApp()) return
  listening = true
  await listen('state', () => void refresh())
  await listen<SigninEvent>('signin', (event) => {
    store.signins[event.payload.accountId] = event.payload
    void refresh()
  })
}

export const addAccount = (input: AccountInput, password: string | null) =>
  invoke<AccountView>('account_add', { input, password }).finally(refresh)
export const updateAccount = (id: string, input: AccountInput, password: string | null) =>
  invoke<AccountView>('account_update', { id, input, password }).finally(refresh)
export const removeAccount = (id: string) => invoke('account_remove', { id }).finally(refresh)
export const setDefault = (id: string) => invoke('account_set_default', { id }).finally(refresh)
export const signIn = (id: string) => {
  delete store.signins[id]
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
/** Toon e-Boekhouden: the hidden sign-in window comes into view. */
export const signinShow = (id: string) => invoke('signin_show', { id })

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
}

/** Initials for the avatar: first letters of the first two words. */
export function initials(label: string): string {
  const words = label.trim().split(/\s+/).filter(Boolean)
  return ((words[0]?.[0] ?? '') + (words[1]?.[0] ?? words[0]?.[1] ?? '')).toUpperCase()
}

/** "2 u 30 min", "12 min", "net". */
export function since(ms: number, now = Date.now()): string {
  const minutes = Math.floor((now - ms) / 60000)
  if (minutes < 1) return 'net'
  const h = Math.floor(minutes / 60)
  const m = minutes % 60
  return h ? `${h} u ${m} min` : `${m} min`
}

export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString('nl-NL', { hour: '2-digit', minute: '2-digit' })
}

export function isRunning(step: Step | undefined): boolean {
  return step !== undefined && !['done', 'failed', 'unknown', 'cancelled'].includes(step)
}
