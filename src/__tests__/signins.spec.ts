import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  addAccount, askedCode, dismissSignin, markStarted, receiveSignin, refresh, signIn, signingIn, startedHere, store,
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

function account(id: string, signingIn: boolean, signin: SigninEvent | null = null): AccountView {
  return {
    id, label: id, username: `${id}@example.test`, store: 'system', opRef: null, opTitle: null, opVault: null,
    administration: null, session: null, signingIn, signin,
  }
}
const event = (accountId: string, step: SigninEvent['step'], message: string | null = null): SigninEvent => ({ accountId, step, message })

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  for (const id of Object.keys(store.signins)) {
    dismissSignin(id)
    delete store.signins[id]
  }
})

describe('sign-ins started in this window', () => {
  it('marks the account when this window starts its sign-in and shows it as running at once', async () => {
    expect(startedHere('a')).toBe(false)
    receiveSignin(event('a', 'done', 'Studio Noord'))
    await signIn('a')
    expect(invoke).toHaveBeenCalledWith('signin_start', { id: 'a' })
    expect(startedHere('a')).toBe(true)
    // Not the end of the sign-in before it.
    expect(store.signins.a).toEqual(event('a', 'credentials'))
  })

  it('shows its own sign-in until it is closed, and keeps the end state known after that', () => {
    markStarted('own')
    receiveSignin(event('own', 'credentials'))
    receiveSignin(event('own', 'done', 'Studio Noord'))
    expect(store.signins.own).toEqual(event('own', 'done', 'Studio Noord'))
    expect(startedHere('own')).toBe(true)
    dismissSignin('own')
    expect(startedHere('own')).toBe(false)
    expect(store.signins.own).toEqual(event('own', 'done', 'Studio Noord'))
  })

  it('keeps the end state of a sign-in from elsewhere, so a window that follows it is not left on a running step', () => {
    receiveSignin(event('other', 'credentials'))
    receiveSignin(event('other', 'administration'))
    expect(signingIn(account('other', true))).toBe(true)
    receiveSignin(event('other', 'done', 'Studio Noord'))
    expect(store.signins.other).toEqual(event('other', 'done', 'Studio Noord'))
    // The newest step decides, even when the account list in hand still says it runs.
    expect(signingIn(account('other', true))).toBe(false)
    expect(startedHere('other')).toBe(false)
  })

  it('remembers that a sign-in from elsewhere asked for a code until it ends', () => {
    receiveSignin(event('mfa', 'code'))
    expect(askedCode('mfa')).toBe(true)
    receiveSignin(event('mfa', 'submit'))
    expect(askedCode('mfa')).toBe(true)
    receiveSignin(event('mfa', 'done'))
    expect(askedCode('mfa')).toBe(false)
    expect(store.signins.mfa?.step).toBe('done')
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
  it('takes each account\'s newest sign-in step from the app, an end state too', async () => {
    invoke.mockResolvedValue([
      account('busy', true, event('busy', 'submit')),
      account('ended', false, event('ended', 'done', 'Studio Noord')),
      account('idle', false),
    ])
    await refresh()
    expect(invoke).toHaveBeenCalledWith('accounts_list')
    expect(store.signins.busy).toEqual(event('busy', 'submit'))
    expect(store.signins.ended).toEqual(event('ended', 'done', 'Studio Noord'))
    expect(store.signins.idle).toBeUndefined()
  })

  it('replaces a step this window was left on with the end the app knows', async () => {
    receiveSignin(event('missed', 'administration'))
    invoke.mockResolvedValue([account('missed', false, event('missed', 'done', 'Studio Noord'))])
    await refresh()
    expect(store.signins.missed).toEqual(event('missed', 'done', 'Studio Noord'))
  })

  it('learns from the list that a running sign-in asked for a code', async () => {
    invoke.mockResolvedValue([account('mfa', true, event('mfa', 'code'))])
    await refresh()
    expect(askedCode('mfa')).toBe(true)
  })

  it('drops a step the app no longer knows', async () => {
    receiveSignin(event('gone', 'failed', 'Wachtwoord klopt niet'))
    invoke.mockResolvedValue([account('gone', false)])
    await refresh()
    expect(store.signins.gone).toBeUndefined()
  })

  it('goes by the account when this window knows no step', () => {
    expect(signingIn(account('fresh', true))).toBe(true)
    expect(signingIn(account('fresh', false))).toBe(false)
  })

  it('lets only the newest refresh write, so an old list cannot undo a newer one', async () => {
    let answerOld: (value: AccountView[]) => void = () => {}
    invoke.mockImplementationOnce(() => new Promise<AccountView[]>((resolve) => (answerOld = resolve)))
    const old = refresh()
    invoke.mockResolvedValueOnce([account('race', false, event('race', 'done'))])
    await refresh()
    expect(store.signins.race?.step).toBe('done')
    answerOld([account('race', true, event('race', 'administration'))])
    await old
    expect(store.accounts[0]?.signingIn).toBe(false)
    expect(store.signins.race?.step).toBe('done')
  })
})
