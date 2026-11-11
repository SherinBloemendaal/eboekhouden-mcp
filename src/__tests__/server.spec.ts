import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { hostPort, inApp, isMac, masked, modKey, server, serverInfo, startupNotice, unlockName, watchServer, type ServerInfo } from '../lib/server'

const invoke = vi.fn()
const unlisten = vi.fn()
let onState: (() => void) | null = null
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: () => void) => {
    if (name === 'state') onState = handler
    return unlisten
  }),
}))

describe('server helpers', () => {
  it('shows only the tail of the token', () => {
    expect(masked('3f9a')).toBe('••••••••••••3f9a')
    expect(masked('')).toBe('••••••••••••')
  })

  it('says there is no server outside the app', async () => {
    expect(inApp()).toBe(false)
    const info = await serverInfo()
    expect(info.running).toBe(false)
    expect(info.url).toBe('http://127.0.0.1:47821/mcp')
    expect(info.toolCount).toBe(0)
    expect(await startupNotice()).toBeNull()
  })

  it('shows the address as host and port', () => {
    expect(hostPort('http://127.0.0.1:47821/mcp')).toBe('127.0.0.1:47821')
    expect(hostPort('http://127.0.0.1:50000/mcp')).toBe('127.0.0.1:50000')
    expect(hostPort('geen adres')).toBe('geen adres')
  })
})

describe('platform names', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.resetModules()
  })

  it('uses the macOS names unless the system is Windows', () => {
    expect(isMac).toBe(true)
    expect(unlockName).toBe('Touch ID')
    expect(modKey).toBe('⌘')
  })

  it('uses Windows Hello and Ctrl+ on Windows', async () => {
    vi.stubGlobal('navigator', { userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36' })
    vi.resetModules()
    const windows = await import('../lib/server')
    expect(windows.isMac).toBe(false)
    expect(windows.unlockName).toBe('Windows Hello')
    expect(windows.modKey).toBe('Ctrl+')
  })
})

describe('watchServer', () => {
  const running: ServerInfo = { url: 'http://127.0.0.1:47821/mcp', tokenTail: 'ab12', running: true, error: null, sessions: 0, toolCount: 12 }
  const failed: ServerInfo = { ...running, running: false, error: 'Poort 47821 is al in gebruik' }

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
    invoke.mockReset()
    unlisten.mockReset()
    onState = null
    server.info = null
    vi.useRealTimers()
  })

  it('fills the state outside the app as well', async () => {
    const stop = watchServer()
    await flushPromises()
    expect(server.info?.running).toBe(false)
    expect(server.info?.error).toBe('Buiten de app')
    stop()
  })

  it('reads again on a state event, on focus and every 30 seconds, and listens once for several callers', async () => {
    // Only the interval is faked: flushPromises needs the real setTimeout and setImmediate.
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] })
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
    invoke.mockResolvedValue(running)
    const stopShell = watchServer()
    const stopPanel = watchServer()
    await flushPromises()
    expect(server.info?.running).toBe(true)
    const reads = () => invoke.mock.calls.filter(([command]) => command === 'server_info').length
    const before = reads()

    // The server failed after the window loaded: the app says so with a state event.
    invoke.mockResolvedValue(failed)
    onState?.()
    await flushPromises()
    expect(server.info).toEqual(failed)
    expect(reads()).toBe(before + 1)

    window.dispatchEvent(new Event('focus'))
    await flushPromises()
    expect(reads()).toBe(before + 2)

    vi.advanceTimersByTime(30_000)
    await flushPromises()
    expect(reads()).toBe(before + 3)

    // One caller leaving keeps the others' updates; the last one stops them all.
    stopShell()
    stopShell()
    window.dispatchEvent(new Event('focus'))
    await flushPromises()
    expect(reads()).toBe(before + 4)
    expect(unlisten).not.toHaveBeenCalled()

    stopPanel()
    expect(unlisten).toHaveBeenCalledTimes(1)
    window.dispatchEvent(new Event('focus'))
    vi.advanceTimersByTime(60_000)
    await flushPromises()
    expect(reads()).toBe(before + 4)
  })

  it('shows the server as off with the reason when the app cannot say', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
    invoke.mockResolvedValueOnce(running)
    const stop = watchServer()
    await flushPromises()
    invoke.mockRejectedValueOnce(new Error('kapot'))
    window.dispatchEvent(new Event('focus'))
    await flushPromises()
    expect(server.info?.running).toBe(false)
    expect(server.info?.error).toContain('kapot')
    expect(server.info?.toolCount).toBe(12)
    stop()
  })

  it('keeps the newest answer when an older read answers later', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
    let answerOld: (info: ServerInfo) => void = () => {}
    invoke.mockImplementationOnce(() => new Promise<ServerInfo>((resolve) => (answerOld = resolve)))
    const stop = watchServer()
    invoke.mockResolvedValueOnce(failed)
    window.dispatchEvent(new Event('focus'))
    await flushPromises()
    expect(server.info).toEqual(failed)
    answerOld(running)
    await flushPromises()
    expect(server.info).toEqual(failed)
    stop()
  })
})
