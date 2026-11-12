import { flushPromises, mount } from '@vue/test-utils'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '../App.vue'
import { isSidebarShortcut, parseSidebar, SIDEBAR_KEY } from '../lib/sidebar'
import { router } from '../router'
import { MemoryStorage } from './memory-storage'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
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
beforeEach(() => {
  localStorage.clear()
  invoke.mockReset()
  invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'approvals_list': return []
      case 'accounts_list': return []
      case 'settings_get': return { idleMinutes: 30, appearance: 'system', showInDock: true }
      case 'log_list': return []
      default: return undefined
    }
  })
})
afterEach(() => {
  for (const wrapper of mounted.splice(0)) wrapper.unmount()
  vi.restoreAllMocks()
  localStorage.clear()
})

// The whole window, as in the app.
async function mountApp() {
  await router.push('/dashboard')
  await router.isReady()
  const wrapper = mount(App, { global: { plugins: [router] }, attachTo: document.body })
  mounted.push(wrapper)
  await flushPromises()
  return wrapper
}

const shell = () => document.querySelector<HTMLElement>('.shell')!
const nav = () => document.querySelector<HTMLElement>('#app-sidebar')!
const handle = () => document.querySelector<HTMLElement>('[role="separator"]')!
const sideButton = () => document.querySelector<HTMLButtonElement>('.sidebar .side-toggle')!
const barButton = () => document.querySelector<HTMLButtonElement>('.toolbar .side-toggle')
const width = () => shell().style.getPropertyValue('--side-w')
const stored = () => JSON.parse(localStorage.getItem(SIDEBAR_KEY) ?? 'null') as { width: number; collapsed: boolean } | null
/** Every link in the sidebar sits inside an inert element, so Tab skips it. */
const linksOutOfReach = () => [...nav().querySelectorAll('a')].every((a) => a.closest('[inert]') !== null)

function pointer(type: string, clientX: number) {
  handle().dispatchEvent(new PointerEvent(type, { clientX, pointerId: 1, button: 0, bubbles: true, cancelable: true }))
}
function key(target: EventTarget, init: KeyboardEventInit) {
  target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }))
}

describe('sidebar width', () => {
  it('starts at 220 pixels, open, with a focusable separator that says so', async () => {
    await mountApp()
    expect(width()).toBe('220px')
    expect(handle().getAttribute('aria-orientation')).toBe('vertical')
    expect(handle().getAttribute('aria-valuemin')).toBe('190')
    expect(handle().getAttribute('aria-valuemax')).toBe('300')
    expect(handle().getAttribute('aria-valuenow')).toBe('220')
    expect(handle().getAttribute('aria-label')).toBe('Breedte van de zijbalk')
    expect(handle().tabIndex).toBe(0)
    expect(nav().hasAttribute('inert')).toBe(false)
    expect(sideButton().getAttribute('aria-expanded')).toBe('true')
    expect(barButton()).toBeNull()
  })

  it('follows a drag within 190 to 300 and stores the width when it is let go', async () => {
    await mountApp()
    pointer('pointerdown', 400)
    await flushPromises()
    expect(shell().classList).toContain('resizing')
    pointer('pointermove', 450)
    await flushPromises()
    expect(width()).toBe('270px')
    // Nothing is written while the hand still moves.
    expect(stored()).toBeNull()
    pointer('pointermove', 900)
    await flushPromises()
    expect(width()).toBe('300px')
    pointer('pointermove', 0)
    await flushPromises()
    expect(width()).toBe('190px')
    pointer('pointermove', 405)
    pointer('pointerup', 405)
    await flushPromises()
    expect(width()).toBe('225px')
    expect(handle().getAttribute('aria-valuenow')).toBe('225')
    expect(shell().classList).not.toContain('resizing')
    expect(stored()).toEqual({ width: 225, collapsed: false })
    // A move after the release does nothing.
    pointer('pointermove', 600)
    await flushPromises()
    expect(width()).toBe('225px')
  })

  it('moves 10 pixels per arrow key, goes to the limits with Home and End, and stores each step', async () => {
    await mountApp()
    handle().focus()
    key(handle(), { key: 'ArrowRight' })
    await flushPromises()
    expect(width()).toBe('230px')
    expect(stored()).toEqual({ width: 230, collapsed: false })
    key(handle(), { key: 'ArrowLeft' })
    key(handle(), { key: 'ArrowLeft' })
    await flushPromises()
    expect(width()).toBe('210px')
    key(handle(), { key: 'Home' })
    key(handle(), { key: 'ArrowLeft' })
    await flushPromises()
    expect(width()).toBe('190px')
    expect(stored()?.width).toBe(190)
    key(handle(), { key: 'End' })
    key(handle(), { key: 'ArrowRight' })
    await flushPromises()
    expect(width()).toBe('300px')
    expect(handle().getAttribute('aria-valuenow')).toBe('300')
    expect(stored()).toEqual({ width: 300, collapsed: false })
  })

  it('goes back to 220 on a double click', async () => {
    localStorage.setItem(SIDEBAR_KEY, JSON.stringify({ width: 280, collapsed: false }))
    await mountApp()
    expect(width()).toBe('280px')
    handle().dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    await flushPromises()
    expect(width()).toBe('220px')
    expect(stored()).toEqual({ width: 220, collapsed: false })
  })
})

describe('sidebar at start-up', () => {
  it('opens with the stored width and the stored collapsed state', async () => {
    localStorage.setItem(SIDEBAR_KEY, JSON.stringify({ width: 260, collapsed: true }))
    await mountApp()
    expect(width()).toBe('260px')
    expect(shell().classList).toContain('collapsed')
    expect(nav().hasAttribute('inert')).toBe(true)
    expect(linksOutOfReach()).toBe(true)
    expect(handle().tabIndex).toBe(-1)
    expect(barButton()).not.toBeNull()
    expect(barButton()!.getAttribute('aria-expanded')).toBe('false')
  })

  it.each([
    ['nothing', null, 220, false],
    ['broken JSON', '{"width": 2', 220, false],
    ['a list', '[260, true]', 220, false],
    ['a string', '"wide"', 220, false],
    ['wrong types', '{"width": "260", "collapsed": "yes"}', 220, false],
    ['no number', '{"width": null, "collapsed": 1}', 220, false],
    ['too wide', '{"width": 999, "collapsed": false}', 300, false],
    ['too narrow', '{"width": 12, "collapsed": true}', 190, true],
    ['half pixels', '{"width": 233.6}', 234, false],
  ])('falls back on %s', async (_name, raw, expectedWidth, expectedCollapsed) => {
    expect(parseSidebar(raw)).toEqual({ width: expectedWidth, collapsed: expectedCollapsed })
    if (raw !== null) localStorage.setItem(SIDEBAR_KEY, raw)
    await mountApp()
    expect(width()).toBe(`${expectedWidth}px`)
    expect(shell().classList.contains('collapsed')).toBe(expectedCollapsed)
  })

  it('opens at 220 when storage refuses to be read, and keeps working when it refuses to be written', async () => {
    localStorage.setItem(SIDEBAR_KEY, JSON.stringify({ width: 260, collapsed: true }))
    vi.spyOn(localStorage, 'getItem').mockImplementation(() => {
      throw new DOMException('blocked', 'SecurityError')
    })
    const setItem = vi.spyOn(localStorage, 'setItem').mockImplementation(() => {
      throw new DOMException('full', 'QuotaExceededError')
    })
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    await mountApp()
    expect(width()).toBe('220px')
    expect(shell().classList).not.toContain('collapsed')
    sideButton().click()
    await flushPromises()
    expect(shell().classList).toContain('collapsed')
    expect(setItem).toHaveBeenCalled()
    expect(warn).toHaveBeenCalledWith('sidebar_save', expect.any(DOMException))
  })
})

describe('sidebar collapse', () => {
  it('collapses with the button, moves the button to the toolbar, and opens again from there', async () => {
    await mountApp()
    expect(sideButton().getAttribute('aria-controls')).toBe('app-sidebar')
    expect(sideButton().getAttribute('aria-label')).toBe('Zijbalk verbergen')
    expect(sideButton().title).toBe('Zijbalk verbergen (⌃⌘S)')
    expect(sideButton().getAttribute('aria-keyshortcuts')).toBe('Control+Meta+S')
    expect(sideButton().querySelector('svg')?.innerHTML).toContain('<rect')
    expect(linksOutOfReach()).toBe(false)

    sideButton().focus()
    sideButton().click()
    await flushPromises()
    expect(shell().classList).toContain('collapsed')
    expect(nav().hasAttribute('inert')).toBe(true)
    expect(linksOutOfReach()).toBe(true)
    expect(handle().tabIndex).toBe(-1)
    expect(stored()).toEqual({ width: 220, collapsed: true })
    const bar = barButton()!
    expect(bar).not.toBeNull()
    expect(bar.getAttribute('aria-expanded')).toBe('false')
    expect(bar.getAttribute('aria-controls')).toBe('app-sidebar')
    expect(bar.getAttribute('aria-label')).toBe('Zijbalk tonen')
    expect(bar.title).toBe('Zijbalk tonen (⌃⌘S)')
    // The focus followed the button out of the sidebar.
    expect(document.activeElement).toBe(bar)
    // The title is still there, beside the button.
    expect(document.querySelector('.toolbar h1')?.textContent).toBe('Dashboard')

    bar.click()
    await flushPromises()
    expect(shell().classList).not.toContain('collapsed')
    expect(nav().hasAttribute('inert')).toBe(false)
    expect(linksOutOfReach()).toBe(false)
    expect(handle().tabIndex).toBe(0)
    expect(sideButton().getAttribute('aria-expanded')).toBe('true')
    expect(document.activeElement).toBe(sideButton())
    expect(barButton()).toBeNull()
    expect(stored()).toEqual({ width: 220, collapsed: false })
  })

  it('toggles with Cmd+Ctrl+S on macOS, also from a text field, and not with Cmd+S or Ctrl+S', async () => {
    await mountApp()
    key(window, { key: 's', metaKey: true })
    key(window, { key: 's', ctrlKey: true })
    key(window, { key: 'S', metaKey: true, ctrlKey: true, shiftKey: true })
    await flushPromises()
    expect(shell().classList).not.toContain('collapsed')

    const event = new KeyboardEvent('keydown', { key: 's', code: 'KeyS', metaKey: true, ctrlKey: true, bubbles: true, cancelable: true })
    window.dispatchEvent(event)
    await flushPromises()
    expect(event.defaultPrevented).toBe(true)
    expect(shell().classList).toContain('collapsed')
    expect(stored()?.collapsed).toBe(true)

    const field = document.createElement('input')
    document.body.append(field)
    field.focus()
    key(field, { key: 's', metaKey: true, ctrlKey: true })
    await flushPromises()
    expect(shell().classList).not.toContain('collapsed')
    expect(document.activeElement).toBe(field)
    field.remove()
  })

  it('keeps the sidebar as it is while a dialog is open', async () => {
    await mountApp()
    const dialog = document.createElement('div')
    dialog.setAttribute('aria-modal', 'true')
    document.body.append(dialog)
    key(window, { key: 's', metaKey: true, ctrlKey: true })
    await flushPromises()
    expect(shell().classList).not.toContain('collapsed')
    dialog.remove()
  })

  it('matches Ctrl+Alt+S on Windows by the key, and leaves it to a text field', () => {
    const ev = (init: KeyboardEventInit) => new KeyboardEvent('keydown', init)
    expect(isSidebarShortcut(ev({ key: 's', code: 'KeyS', ctrlKey: true, altKey: true }), false)).toBe(true)
    // AltGr+S on US International gives ß: the key's place still counts outside a field.
    expect(isSidebarShortcut(ev({ key: 'ß', code: 'KeyS', ctrlKey: true, altKey: true }), false)).toBe(true)
    expect(isSidebarShortcut(ev({ key: 's', code: 'KeyS', ctrlKey: true }), false)).toBe(false)
    expect(isSidebarShortcut(ev({ key: 's', code: 'KeyS', metaKey: true, ctrlKey: true }), false)).toBe(false)
    expect(isSidebarShortcut(ev({ key: 's', code: 'KeyS', metaKey: true, ctrlKey: true }), true)).toBe(true)
    expect(isSidebarShortcut(ev({ key: 's', code: 'KeyS', metaKey: true, ctrlKey: true, repeat: true }), true)).toBe(false)
  })
})
