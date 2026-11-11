import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { reactive } from 'vue'

export interface ServerInfo {
  url: string
  tokenTail: string
  running: boolean
  error: string | null
  sessions: number
  /** How many tools the MCP server offers. */
  toolCount: number
}

const userAgent = typeof navigator === 'undefined' ? '' : navigator.userAgent

/** macOS unless the webview says Windows (the app ships for those two). */
export const isMac: boolean = !/Windows/i.test(userAgent)
/** What unlocks the stored password: Touch ID on macOS, Windows Hello on Windows. */
export const unlockName: string = isMac ? 'Touch ID' : 'Windows Hello'
/** The modifier in shortcut hints: "⌘," on macOS, "Ctrl+," on Windows. */
export const modKey: string = isMac ? '⌘' : 'Ctrl+'

/** True inside the Tauri app; false in a plain browser (vite dev) and in tests. */
export const inApp = (): boolean => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** What the local server is doing. Outside the app there is no server, and it says so. */
export async function serverInfo(): Promise<ServerInfo> {
  if (!inApp()) {
    return { url: 'http://127.0.0.1:47821/mcp', tokenTail: '', running: false, error: 'Buiten de app', sessions: 0, toolCount: 0 }
  }
  return invoke<ServerInfo>('server_info')
}

/** "127.0.0.1:47821" from "http://127.0.0.1:47821/mcp"; the address as given when it is no URL. */
export function hostPort(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return url
  }
}

/** Something EB MCP noticed at startup, such as a damaged accounts.json it set aside. Null when all was well. */
export const startupNotice = (): Promise<string | null> =>
  inApp() ? invoke<string | null>('startup_notice') : Promise.resolve(null)

/** The server's state for this window, kept current by watchServer(). */
export const server = reactive({ info: null as ServerInfo | null })

const SERVER_POLL_MS = 30_000
let readSeq = 0

/** Reads the server's state now; an older read that answers later never overwrites a newer one. */
export async function readServer(): Promise<void> {
  const seq = ++readSeq
  let info: ServerInfo
  try {
    info = await serverInfo()
  } catch (error) {
    // The app could not say: show the server as off, with the reason, rather than an old "actief".
    info = { ...(server.info ?? emptyInfo()), running: false, error: String(error) }
  }
  if (seq === readSeq) server.info = info
}

function emptyInfo(): ServerInfo {
  return { url: '', tokenTail: '', running: false, error: null, sessions: 0, toolCount: 0 }
}

let watchers = 0
let unwatch: (() => void) | null = null

/**
 * Keeps `server.info` current: on a `state` event (the app sends one when the server fails to start), when the
 * window gains focus, and every 30 seconds. Safe to call from several components: the window listens once, and the
 * listeners stop when the last caller calls the returned function.
 */
export function watchServer(): () => void {
  watchers += 1
  if (watchers === 1) unwatch = startWatching()
  else void readServer()
  let stopped = false
  return () => {
    if (stopped) return
    stopped = true
    watchers -= 1
    if (watchers === 0) {
      unwatch?.()
      unwatch = null
    }
  }
}

function startWatching(): () => void {
  const read = () => void readServer()
  read()
  window.addEventListener('focus', read)
  const timer = setInterval(read, SERVER_POLL_MS)
  let unlisten: (() => void) | null = null
  let stopped = false
  if (inApp()) {
    void listen('state', read).then((stop) => {
      if (stopped) stop()
      else unlisten = stop
    })
  }
  return () => {
    stopped = true
    window.removeEventListener('focus', read)
    clearInterval(timer)
    unlisten?.()
  }
}

export async function openMain(route?: string): Promise<void> {
  if (inApp()) await invoke('open_main', { route })
}

/** How many MCP calls are still running; quitting asks first when this is above zero. */
export const runningCalls = (): Promise<number> => (inApp() ? invoke<number>('running_calls') : Promise.resolve(0))

/** Opens e-Boekhouden's sign-in page in the default browser, for a step EB MCP does not know. */
export async function openEboekhouden(): Promise<void> {
  if (inApp()) await invoke('open_eboekhouden')
}

/** Sizes the menu-bar panel to its content (logical pixels; the app keeps it between limits). */
export async function fitPanel(height: number): Promise<void> {
  if (inApp()) await invoke('panel_fit', { height })
}

/** Whether the system opens EB MCP at login (its own login item, not a setting of ours). */
export const autostartGet = (): Promise<boolean> => (inApp() ? invoke<boolean>('autostart_get') : Promise.resolve(false))
export const autostartSet = (enabled: boolean): Promise<boolean> => invoke<boolean>('autostart_set', { enabled })

/** Opens the folder with EB MCP's log files. */
export const openLogDir = () => (inApp() ? invoke('open_log_dir') : Promise.resolve())

export async function quitApp(): Promise<void> {
  if (inApp()) await invoke('quit')
}

/** Shows only the last characters of a secret, the way the boards do. */
export function masked(tail: string): string {
  return tail ? `••••••••••••${tail}` : '••••••••••••'
}
