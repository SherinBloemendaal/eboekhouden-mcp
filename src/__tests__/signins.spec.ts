import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  addAccount, askedCode, dismissSignin, markStarted, receiveSignin, refresh, signIn, startedHere, store,
  type AccountView, type SigninEvent,
} from '../lib/accounts'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))

// refresh() only asks the app for accounts inside the app.
beforeAll(() => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
})
afterAll(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

function account(id: string, signingIn: boolean): AccountView {
  return {
    id, label: id, username: `${id}@example.test`, store: 'system', opRef: null, opTitle: null, opVault: null,
    administration: null, session: null, signingIn,
  }
}
const event = (accountId: string, step: SigninEvent['step'], message: string | null = null): SigninEvent => ({ accountId, step, message })

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  for (const id of Object.keys(store.signins)) dismissSignin(id)
})

describe('sign-ins started in this window', () => {
  it('marks the account when this window starts its sign-in', async () => {
    expect(startedHere('a')).toBe(false)
    await signIn('a')
    expect(invoke).toHaveBeenCalledWith('signin_start', { id: 'a' })
    expect(startedHere('a')).toBe(true)
  })

  it('keeps the end state of its own sign-in until it is closed', () => {
    markStarted('own')
    receiveSignin(event('own', 'credentials'))
    receiveSignin(event('own', 'done', 'Studio Noord'))
    expect(store.signins.own).toEqual(event('own', 'done', 'Studio Noord'))
    // The end frees the mark: a later sign-in from another window is not this window's.
    expect(startedHere('own')).toBe(false)
    dismissSignin('own')
    expect(store.signins.own).toBeUndefined()
  })

  it('shows a sign-in from elsewhere as running, then drops it when it ends', () => {
    receiveSignin(event('other', 'credentials'))
    expect(store.signins.other?.step).toBe('credentials')
    receiveSignin(event('other', 'submit'))
    expect(store.signins.other?.step).toBe('submit')
    receiveSignin(event('other', 'failed', 'Wachtwoord klopt niet'))
    expect(store.signins.other).toBeUndefined()
  })

  it('remembers that a sign-in from elsewhere asked for a code until it ends', () => {
    receiveSignin(event('mfa', 'code'))
    expect(askedCode('mfa')).toBe(true)
    receiveSignin(event('mfa', 'submit'))
    expect(askedCode('mfa')).toBe(true)
    receiveSignin(event('mfa', 'done'))
    expect(askedCode('mfa')).toBe(false)
    expect(store.signins.mfa).toBeUndefined()
  })

  it('marks an account it added, since the app signs that one in by itself', async () => {
    invoke.mockImplementation(async (command: string) => (command === 'account_add' ? account('new', true) : []))
    await addAccount({ label: 'new', username: 'n', store: 'system', opRef: null, opTitle: null, opVault: null }, 'geheim')
    expect(startedHere('new')).toBe(true)
    receiveSignin(event('new', 'done'))
    expect(store.signins.new?.step).toBe('done')
  })
})

describe('refresh', () => {
  it('shows a sign-in that was already running when the window loaded', async () => {
    invoke.mockResolvedValue([account('busy', true), account('idle', false)])
    await refresh()
    expect(invoke).toHaveBeenCalledWith('accounts_list')
    expect(store.signins.busy).toEqual({ accountId: 'busy', step: 'submit', message: null })
    expect(store.signins.idle).toBeUndefined()
  })

  it('keeps a stored step of a running sign-in', async () => {
    receiveSignin(event('busy', 'code'))
    invoke.mockResolvedValue([account('busy', true)])
    await refresh()
    expect(store.signins.busy?.step).toBe('code')
  })

  it('removes a running step once the sign-in stopped, but keeps an end state', async () => {
    receiveSignin(event('stale', 'submit'))
    markStarted('ended')
    receiveSignin(event('ended', 'done'))
    invoke.mockResolvedValue([account('stale', false), account('ended', false)])
    await refresh()
    expect(store.signins.stale).toBeUndefined()
    expect(store.signins.ended?.step).toBe('done')
  })

  it('lets only the newest refresh write, so an old list cannot undo a newer one', async () => {
    let answerOld: (value: AccountView[]) => void = () => {}
    invoke.mockImplementationOnce(() => new Promise<AccountView[]>((resolve) => (answerOld = resolve)))
    const old = refresh()
    invoke.mockResolvedValueOnce([account('race', true)])
    await refresh()
    expect(store.signins.race?.step).toBe('submit')
    answerOld([account('race', false)])
    await old
    expect(store.accounts[0]?.signingIn).toBe(true)
    expect(store.signins.race?.step).toBe('submit')
  })
})
