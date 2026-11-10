import { invoke } from '@tauri-apps/api/core'

export interface ServerInfo {
  url: string
  tokenTail: string
  running: boolean
  error: string | null
  sessions: number
}

/** True inside the Tauri app; false in a plain browser (vite dev) and in tests. */
export const inApp = (): boolean => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** What the local server is doing. Outside the app there is no server, and it says so. */
export async function serverInfo(): Promise<ServerInfo> {
  if (!inApp()) {
    return { url: 'http://127.0.0.1:47821/mcp', tokenTail: '', running: false, error: 'Buiten de app', sessions: 0 }
  }
  return invoke<ServerInfo>('server_info')
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

export async function quitApp(): Promise<void> {
  if (inApp()) await invoke('quit')
}

/** Shows only the last characters of a secret, the way the boards do. */
export function masked(tail: string): string {
  return tail ? `••••••••••••${tail}` : '••••••••••••'
}
