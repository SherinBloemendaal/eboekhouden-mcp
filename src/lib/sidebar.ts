import { reactive } from 'vue'
import { isMac } from './server'

/**
 * The main window's sidebar: its width and whether it is collapsed, as the user left them. Kept in the webview's
 * localStorage under one key: it is a preference of this window only, it must be there before the first paint (the
 * app's own settings come from the Rust side, asynchronously), and the webview keeps its storage across restarts and
 * updates because the app's identifier and origin stay the same.
 */
export const SIDEBAR_MIN = 190
export const SIDEBAR_MAX = 300
export const SIDEBAR_DEFAULT = 220
/** How far one arrow key moves the handle. */
export const SIDEBAR_STEP = 10
export const SIDEBAR_KEY = 'eb-mcp.sidebar'

export interface SidebarState {
  width: number
  collapsed: boolean
}

export const sidebar = reactive<SidebarState>({ width: SIDEBAR_DEFAULT, collapsed: false })

/** A width the sidebar may have: whole pixels between the minimum and the maximum. */
export function clampWidth(width: number): number {
  return Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, Math.round(width)))
}

/**
 * What a stored value means. Missing or unreadable: the default width, open. A width that is no finite number: the
 * default width. A width outside the limits: the nearest limit. Collapsed only when it says exactly true.
 */
export function parseSidebar(raw: string | null): SidebarState {
  const fallback = { width: SIDEBAR_DEFAULT, collapsed: false }
  if (!raw) return fallback
  let value: unknown
  try {
    value = JSON.parse(raw)
  } catch {
    return fallback
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return fallback
  const { width, collapsed } = value as Record<string, unknown>
  return {
    width: typeof width === 'number' && Number.isFinite(width) ? clampWidth(width) : SIDEBAR_DEFAULT,
    collapsed: collapsed === true,
  }
}

function storage(): Storage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage
  } catch {
    // Storage the webview refuses to give (blocked): the sidebar works, it is only not remembered.
    return null
  }
}

/** Reads the stored state into `sidebar`. The shell calls it once while it is set up, before its first render. */
export function loadSidebar(): void {
  let raw: string | null = null
  try {
    raw = storage()?.getItem(SIDEBAR_KEY) ?? null
  } catch {
    raw = null
  }
  Object.assign(sidebar, parseSidebar(raw))
}

/** Stores the current state. A refusal is logged and otherwise ignored: the window keeps working as it is. */
export function saveSidebar(): void {
  try {
    storage()?.setItem(SIDEBAR_KEY, JSON.stringify({ width: sidebar.width, collapsed: sidebar.collapsed }))
  } catch (error) {
    console.warn('sidebar_save', error)
  }
}

export function setSidebarWidth(width: number): void {
  sidebar.width = clampWidth(width)
  saveSidebar()
}

export function setSidebarCollapsed(collapsed: boolean): void {
  sidebar.collapsed = collapsed
  saveSidebar()
}

/** The shortcut as the button's tooltip shows it. */
export const sidebarShortcut: string = isMac ? '⌃⌘S' : 'Ctrl+Alt+S'
/** The shortcut for aria-keyshortcuts. */
export const sidebarShortcutAria: string = isMac ? 'Control+Meta+S' : 'Control+Alt+S'

/** True when the focus is in a field where keys type text. */
function typesText(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  if (target.isContentEditable || target instanceof HTMLTextAreaElement) return true
  if (!(target instanceof HTMLInputElement)) return false
  return !['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'color', 'file', 'image'].includes(target.type)
}

/**
 * Cmd+Ctrl+S on macOS, Ctrl+Alt+S on Windows. On macOS a combination with Cmd never types a character, so it works in
 * a text field too. On Windows Ctrl+Alt is AltGr, and AltGr+S types a character on some layouts (ß on US
 * International, ś on Polish): there it is left to the text field. Outside a field it goes by the key's place
 * (KeyS), so it also works on a layout where AltGr+S would give another character.
 */
export function isSidebarShortcut(event: KeyboardEvent, mac: boolean = isMac): boolean {
  if (event.repeat || event.shiftKey) return false
  if (mac) return event.metaKey && event.ctrlKey && !event.altKey && event.key.toLowerCase() === 's'
  return event.ctrlKey && event.altKey && !event.metaKey && event.code === 'KeyS' && !typesText(event.target)
}
