import { flushPromises, mount } from '@vue/test-utils'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '../App.vue'
import { createGlassSync, type GlassAnswer, type GlassRect } from '../lib/glass'
import { router } from '../router'
import { MemoryStorage } from './memory-storage'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))

/** What the app answers to set_glass in the next test; a function may throw to make the call fail. */
let glassAnswer: () => GlassAnswer = () => ({ supported: true, shapes: 1 })
/** Where the card is, as the layout would put it; jsdom has no layout of its own. */
let cardBox = { left: 8, top: 8, width: 202, height: 764 }

/** Animation frames run when the test says so, and the clock is the test's. */
let frames: (() => void)[] = []
let clock = 0
function runFrames(count: number, step = 16) {
  for (let i = 0; i < count && frames.length; i++) {
    clock += step
    const due = frames.splice(0)
    for (const callback of due) callback()
  }
}

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
  glassAnswer = () => ({ supported: true, shapes: 1 })
  cardBox = { left: 8, top: 8, width: 202, height: 764 }
  frames = []
  clock = 0
  vi.stubGlobal('requestAnimationFrame', (callback: () => void) => frames.push(callback))
  vi.spyOn(performance, 'now').mockImplementation(() => clock)
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.id !== 'app-sidebar') return new DOMRect(0, 0, 0, 0)
    return new DOMRect(cardBox.left, cardBox.top, cardBox.width, cardBox.height)
  })
  invoke.mockReset()
  invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'set_glass': return glassAnswer()
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

async function mountApp() {
  await router.push('/dashboard')
  await router.isReady()
  const wrapper = mount(App, { global: { plugins: [router] }, attachTo: document.body })
  mounted.push(wrapper)
  await flushPromises()
  return wrapper
}

const shell = () => document.querySelector<HTMLElement>('.shell')!
const glassCalls = () => invoke.mock.calls.filter((call) => call[0] === 'set_glass').map((call) => call[1] as { viewportWidth: number; rects: GlassRect[] })
const lastShapes = () => glassCalls().at(-1)?.rects
const card = (width = 202, left = 8): GlassRect => ({ id: 'sidebar', x: left, y: 8, width, height: 764, radius: 18 })

describe('native glass in the main window', () => {
  it('reports the card with radius 18 and makes it clear only after the app says the glass is there', async () => {
    let answer!: (value: GlassAnswer) => void
    const pending = new Promise<GlassAnswer>((resolve) => (answer = resolve))
    const base = invoke.getMockImplementation()!
    invoke.mockImplementation((command: string, args?: unknown) => (command === 'set_glass' ? pending : base(command, args)))

    await mountApp()
    expect(glassCalls()).toEqual([{ viewportWidth: window.innerWidth, rects: [card()] }])
    expect(shell().classList.contains('native-glass')).toBe(false)

    answer({ supported: true, shapes: 1 })
    await flushPromises()
    expect(shell().classList.contains('native-glass')).toBe(true)
  })

  it('keeps the CSS glass when the app says this macOS has none, and stops asking', async () => {
    glassAnswer = () => ({ supported: false, shapes: 0 })
    await mountApp()
    expect(glassCalls()).toHaveLength(1)
    expect(shell().classList.contains('native-glass')).toBe(false)

    cardBox.width = 240
    window.dispatchEvent(new Event('resize'))
    await flushPromises()
    expect(glassCalls()).toHaveLength(1)
    expect(shell().classList.contains('native-glass')).toBe(false)
  })

  it('keeps the CSS glass when the call fails, and takes it back when a later call fails', async () => {
    glassAnswer = () => {
      throw new Error('Het glas kon niet worden bijgewerkt.')
    }
    await mountApp()
    expect(shell().classList.contains('native-glass')).toBe(false)

    // The next change tries again; this time it works.
    glassAnswer = () => ({ supported: true, shapes: 1 })
    cardBox.width = 212
    window.dispatchEvent(new Event('resize'))
    await flushPromises()
    expect(lastShapes()).toEqual([card(212)])
    expect(shell().classList.contains('native-glass')).toBe(true)

    glassAnswer = () => {
      throw new Error('Het venster antwoordde niet.')
    }
    cardBox.width = 222
    window.dispatchEvent(new Event('resize'))
    await flushPromises()
    expect(shell().classList.contains('native-glass')).toBe(false)
  })

  it('follows a drag of the handle', async () => {
    await mountApp()
    const handle = document.querySelector<HTMLElement>('[role="separator"]')!
    const pointer = (type: string, clientX: number) =>
      handle.dispatchEvent(new PointerEvent(type, { clientX, pointerId: 1, button: 0, bubbles: true, cancelable: true }))
    pointer('pointerdown', 220)
    cardBox.width = 232
    pointer('pointermove', 250)
    await flushPromises()
    expect(lastShapes()).toEqual([card(232)])
    cardBox.width = 262
    pointer('pointermove', 280)
    await flushPromises()
    expect(lastShapes()).toEqual([card(262)])
    pointer('pointerup', 280)
  })

  it('follows the card every frame while it collapses, then reports no shape at all', async () => {
    await mountApp()
    const before = glassCalls().length
    document.querySelector<HTMLButtonElement>('.sidebar .side-toggle')!.click()
    await flushPromises()

    // Halfway: narrower and closer to the edge, still a shape.
    cardBox = { left: 4, top: 8, width: 101, height: 764 }
    runFrames(1)
    await flushPromises()
    expect(lastShapes()).toEqual([card(101, 4)])

    cardBox = { left: 0, top: 8, width: 0, height: 764 }
    runFrames(30)
    await flushPromises()
    expect(lastShapes()).toEqual([])
    expect(glassCalls().length).toBeGreaterThan(before + 1)
    // Settled: nothing more is asked for.
    expect(frames).toHaveLength(0)

    // Open again: the shape comes back while it slides in.
    document.querySelector<HTMLButtonElement>('.toolbar .side-toggle')!.click()
    await flushPromises()
    cardBox = { left: 8, top: 8, width: 202, height: 764 }
    runFrames(30)
    await flushPromises()
    expect(lastShapes()).toEqual([card()])
  })

  it('reports nothing for a sidebar that starts collapsed', async () => {
    localStorage.setItem('eb-mcp.sidebar', JSON.stringify({ width: 220, collapsed: true }))
    cardBox = { left: 0, top: 8, width: 0, height: 764 }
    await mountApp()
    expect(glassCalls()).toEqual([{ viewportWidth: window.innerWidth, rects: [] }])
    expect(shell().classList.contains('native-glass')).toBe(true)
  })
})

describe('the selected row when the window has no focus', () => {
  it('marks the shell inactive on blur and active again on focus', async () => {
    await mountApp()
    window.dispatchEvent(new Event('blur'))
    await flushPromises()
    expect(shell().classList.contains('inactive')).toBe(true)
    window.dispatchEvent(new Event('focus'))
    await flushPromises()
    expect(shell().classList.contains('inactive')).toBe(false)
  })
})

describe('createGlassSync', () => {
  function setup(answers: (() => Promise<GlassAnswer>)[] = []) {
    const sent: GlassRect[][] = []
    const states: boolean[] = []
    let collapsed = false
    const element = document.createElement('nav')
    element.id = 'app-sidebar'
    const send = vi.fn((rects: GlassRect[]) => {
      sent.push(rects)
      return (answers.shift() ?? (() => Promise.resolve({ supported: true, shapes: rects.length })))()
    })
    const sync = createGlassSync({
      card: () => element,
      collapsed: () => collapsed,
      onSupported: (on) => states.push(on),
      send,
      frame: (callback) => frames.push(callback),
      now: () => clock,
    })
    return { sync, sent, states, send, collapse: (on: boolean) => (collapsed = on) }
  }

  it('has one call in flight and sends the newest place when it returns', async () => {
    let release!: () => void
    const slow = () => new Promise<GlassAnswer>((resolve) => (release = () => resolve({ supported: true, shapes: 1 })))
    const { sync, sent, states } = setup([slow])
    sync.sync()
    cardBox.width = 210
    sync.sync()
    cardBox.width = 220
    sync.sync()
    expect(sent).toHaveLength(1)
    release()
    await flushPromises()
    expect(sent).toEqual([[card()], [card(220)]])
    expect(states).toEqual([true, true])
  })

  it('sends nothing when nothing moved', async () => {
    const { sync, sent } = setup()
    sync.sync()
    await flushPromises()
    sync.sync()
    await flushPromises()
    expect(sent).toHaveLength(1)
  })

  it('takes the shape away on stop, after a call still in flight', async () => {
    let release!: () => void
    const slow = () => new Promise<GlassAnswer>((resolve) => (release = () => resolve({ supported: true, shapes: 1 })))
    const { sync, sent, states } = setup([slow])
    sync.sync()
    sync.stop()
    expect(sent).toHaveLength(1)
    release()
    await flushPromises()
    expect(sent).toEqual([[card()], []])
    // A stopped page is not told anything.
    expect(states).toEqual([])
  })

  it('reports the card while a collapse runs and none once it has settled', async () => {
    const { sync, sent, collapse } = setup()
    sync.sync()
    await flushPromises()
    collapse(true)
    sync.follow(300)
    cardBox.width = 50
    runFrames(1)
    await flushPromises()
    expect(sent.at(-1)).toEqual([card(50)])
    runFrames(30)
    await flushPromises()
    expect(sent.at(-1)).toEqual([])
    expect(frames).toHaveLength(0)
  })
})
