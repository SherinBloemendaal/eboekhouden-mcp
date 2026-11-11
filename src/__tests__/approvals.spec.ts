import { flushPromises, mount } from '@vue/test-utils'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import AccountForm from '../components/AccountForm.vue'
import ApprovalSheet from '../components/ApprovalSheet.vue'
import { store, type AccountView, type Proposal } from '../lib/accounts'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))

beforeAll(() => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
})
afterAll(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

function proposal(over: Partial<Proposal> = {}): Proposal {
  return {
    id: 7, accountId: 'a1', account: 'Studio Noord', client: 'Claude', tool: 'mutatie_toevoegen',
    title: 'Mutatie toevoegen: Geld uitgegeven',
    lines: [{ label: 'Rekening', value: '1010 Bank' }, { label: 'Regel 1', value: '4550 Bankkosten: 2,00 (Geen btw)' }],
    danger: false, askedAt: 1, ...over,
  }
}

/** What the app answers: the waiting proposals for the list, nothing for the rest. */
let waiting: Proposal[] = []
const mounted: { unmount(): void }[] = []
beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (command: string, args?: { id: number }) => {
    if (command === 'approvals_list') return waiting
    if (command === 'approval_decide') waiting = waiting.filter((p) => p.id !== args?.id)
    if (command === 'accounts_list') return []
    return undefined
  })
})
afterEach(() => {
  for (const wrapper of mounted.splice(0)) wrapper.unmount()
  store.approvals = []
  waiting = []
})

async function sheet(list: Proposal[]) {
  waiting = list
  const wrapper = mount(ApprovalSheet, { attachTo: document.body })
  mounted.push(wrapper)
  await flushPromises()
  return wrapper
}
const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]')
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((b) => b.textContent?.trim() === text)!

describe('a change that waits for approval', () => {
  it('shows nothing while nothing waits', async () => {
    await sheet([])
    expect(dialog()).toBeNull()
  })

  it('shows what will change, who asks and for which account', async () => {
    await sheet([proposal()])
    expect(dialog()?.getAttribute('aria-label')).toBe('Mutatie toevoegen: Geld uitgegeven')
    const text = dialog()!.textContent!
    expect(text).toContain('Studio Noord')
    expect(text).toContain('Claude wil dit wijzigen in e-Boekhouden')
    expect([...dialog()!.querySelectorAll('dt')].map((e) => e.textContent)).toEqual(['Rekening', 'Regel 1'])
    expect([...dialog()!.querySelectorAll('dd')].map((e) => e.textContent)).toEqual(['1010 Bank', '4550 Bankkosten: 2,00 (Geen btw)'])
    expect(button('Goedkeuren').classList.contains('primary')).toBe(true)
    expect(text).not.toContain('niet terug te draaien')
  })

  it('sends the change only on Goedkeuren, and then shows the next one', async () => {
    await sheet([proposal(), proposal({ id: 8, title: 'Factuur F00012 verwijderen', danger: true })])
    expect(dialog()!.textContent).toContain('wacht nog 1 wijziging')
    button('Goedkeuren').click()
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('approval_decide', { id: 7, approve: true })
    // The next one cannot be undone: it says so, and its button is the destructive one.
    expect(dialog()?.getAttribute('aria-label')).toBe('Factuur F00012 verwijderen')
    expect(dialog()!.textContent).toContain('niet terug te draaien')
    expect(button('Goedkeuren').classList.contains('danger-fill')).toBe(true)
  })

  it('refuses on Weigeren and on Esc, never approves by accident', async () => {
    await sheet([proposal(), proposal({ id: 8 })])
    button('Weigeren').click()
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('approval_decide', { id: 7, approve: false })
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('approval_decide', { id: 8, approve: false })
    expect(invoke).not.toHaveBeenCalledWith('approval_decide', expect.objectContaining({ approve: true }))
    expect(dialog()).toBeNull()
  })

  it('says so when the proposal was already gone', async () => {
    await sheet([proposal()])
    invoke.mockImplementation(async (command: string) => {
      if (command === 'approval_decide') throw 'Dit voorstel wacht niet meer: het is al beantwoord of verlopen'
      return command === 'approvals_list' ? waiting : undefined
    })
    button('Goedkeuren').click()
    await flushPromises()
    expect(dialog()!.querySelector('[role="alert"]')?.textContent).toContain('wacht niet meer')
  })
})

describe('the write setting of an account', () => {
  const account: AccountView = {
    id: 'a1', label: 'Studio Noord', username: 'jan@studionoord.example', store: 'system', opRef: null, opTitle: null, opVault: null,
    administration: 'Studio Noord B.V.', write: 'off', session: null, signingIn: false, signin: null,
  }

  it('is read-only until the user chooses otherwise, and saves the choice', async () => {
    const wrapper = mount(AccountForm, { props: { account }, attachTo: document.body })
    mounted.push(wrapper)
    await flushPromises()
    const group = document.querySelector('[role="radiogroup"][aria-label="AI-clients mogen in dit account"]')!
    const options = [...group.querySelectorAll<HTMLButtonElement>('[role="radio"]')]
    expect(options.map((o) => o.textContent)).toEqual(['Alleen lezen', 'Boeken met akkoord', 'Boeken zonder akkoord'])
    expect(options.map((o) => o.getAttribute('aria-checked'))).toEqual(['true', 'false', 'false'])
    expect(group.textContent).toContain('niets wijzigen')
    options[1]!.click()
    await flushPromises()
    expect(options[1]!.getAttribute('aria-checked')).toBe('true')
    expect(group.textContent).toContain('pas na Goedkeuren')
    invoke.mockImplementation(async (command: string) => (command === 'account_update' ? { ...account, write: 'ask' } : []))
    document.querySelector<HTMLFormElement>('#account-form')!.requestSubmit()
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('account_update', expect.objectContaining({ id: 'a1', input: expect.objectContaining({ write: 'ask' }) }))
  })
})
