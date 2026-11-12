import { flushPromises, mount } from '@vue/test-utils'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '../App.vue'
import { SIDEBAR_KEY } from '../lib/sidebar'
import { router } from '../router'
import { MemoryStorage } from './memory-storage'

// The same window as on Windows: lib/server decides the platform from the webview once, so it is replaced here.
vi.mock('../lib/server', async (original) => ({ ...(await original<typeof import('../lib/server')>()), isMac: false, modKey: 'Ctrl+' }))
const invoke = vi.fn(async (command: string) => {
  switch (command) {
    case 'approvals_list': return []
    case 'accounts_list': return []
    case 'settings_get': return { idleMinutes: 30, appearance: 'system', showInDock: true }
    case 'log_list': return []
    default: return undefined
  }
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: [string]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))

beforeAll(() => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
  vi.stubGlobal('localStorage', new MemoryStorage())
})
afterAll(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
  vi.unstubAllGlobals()
})

const mounted: { unmount(): void }[] = []
beforeEach(() => localStorage.clear())
afterEach(() => {
  for (const wrapper of mounted.splice(0)) wrapper.unmount()
  localStorage.clear()
})

async function mountApp() {
  await router.push('/dashboard')
  await router.isReady()
  const wrapper = mount(App, { global: { plugins: [router] }, attachTo: document.body })
  mounted.push(wrapper)
  await flushPromises()
  return wrapper
}

const collapsed = () => document.querySelector('.shell')!.classList.contains('collapsed')
const press = (target: EventTarget, init: KeyboardEventInit) =>
  target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }))

describe('sidebar on Windows', () => {
  it('toggles with Ctrl+Alt+S, not with Cmd+Ctrl+S, and not while a text field has the focus', async () => {
    await mountApp()
    const button = document.querySelector<HTMLButtonElement>('.sidebar .side-toggle')!
    expect(button.title).toBe('Zijbalk verbergen (Ctrl+Alt+S)')
    expect(button.getAttribute('aria-keyshortcuts')).toBe('Control+Alt+S')

    press(window, { key: 's', code: 'KeyS', metaKey: true, ctrlKey: true })
    await flushPromises()
    expect(collapsed()).toBe(false)

    press(window, { key: 's', code: 'KeyS', ctrlKey: true, altKey: true })
    await flushPromises()
    expect(collapsed()).toBe(true)
    expect(JSON.parse(localStorage.getItem(SIDEBAR_KEY)!)).toEqual({ width: 220, collapsed: true })
    expect(document.querySelector<HTMLButtonElement>('.toolbar .side-toggle')?.title).toBe('Zijbalk tonen (Ctrl+Alt+S)')

    // AltGr is Ctrl+Alt: in a field it may type a character (ß on US International), so the field keeps it.
    const field = document.createElement('textarea')
    document.body.append(field)
    field.focus()
    press(field, { key: 'ß', code: 'KeyS', ctrlKey: true, altKey: true })
    await flushPromises()
    expect(collapsed()).toBe(true)
    field.remove()

    press(window, { key: 's', code: 'KeyS', ctrlKey: true, altKey: true })
    await flushPromises()
    expect(collapsed()).toBe(false)
  })
})
