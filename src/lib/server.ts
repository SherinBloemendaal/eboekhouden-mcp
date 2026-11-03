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

export async function quitApp(): Promise<void> {
  if (inApp()) await invoke('quit')
}

/** Shows only the last characters of a secret, the way the boards do. */
export function masked(tail: string): string {
  return tail ? `••••••••••••${tail}` : '••••••••••••'
}
