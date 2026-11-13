import { flushPromises, mount } from '@vue/test-utils'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '../App.vue'
import AccountForm from '../components/AccountForm.vue'
import { rightsChip, rightsText, store, type AccountView } from '../lib/accounts'
import {
  ago, approvalState, decisionLabel, filterHistory, followSelection, nameList, proposalKind, quitWarning, refreshApprovals, splitTitle, statusOf, type ApprovalDetail, type ApprovalRecord, type DecideAllResult, type Proposal,
} from '../lib/approvals'
import { statusLabel, type McpEntry } from '../lib/log'
import { router } from '../router'

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
    lines: [{ label: 'Rekening', value: '1010 Bank' }, { label: 'Omschrijving', value: 'Regel een\nRegel twee' }],
    danger: false, askedAt: Date.now() - 5 * 60_000, ...over,
  }
}

function record(over: Partial<ApprovalRecord> = {}): ApprovalRecord {
  return { ...proposal(), decidedAt: null, decision: null, status: 'waiting', error: null, version: '0.4.0', bulk: false, ...over }
}

/** A promise the test settles itself, to look at the page while a command still runs. */
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}

/** What the app answers. */
let waiting: Proposal[] = []
let history: ApprovalRecord[] = []
let onDecide: (id: number, approve: boolean) => Promise<void> = async (id) => {
  waiting = waiting.filter((p) => p.id !== id)
}
let onDecideAll: (ids: number[], approve: boolean) => Promise<DecideAllResult> = async (ids) => {
  waiting = waiting.filter((p) => !ids.includes(p.id))
  return { decided: ids, gone: [] }
}
let onGet: (id: number) => Promise<ApprovalDetail> = async (id) => ({
  ...history.find((r) => r.id === id)!, args: { omschrijving: 'Bankkosten' }, body: { Rekening: '1010' }, result: { MutatieNr: 812 },
})
let onExport: () => Promise<string> = async () => '/Users/jan/Downloads/eb-mcp-akkoord.jsonl.gz'
let onReveal: (path: string) => Promise<void> = async () => {}

const mounted: { unmount(): void }[] = []
beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case 'approvals_list': return waiting
      case 'approval_decide': return onDecide(args!.id as number, args!.approve as boolean)
      case 'approvals_decide_all': return onDecideAll(args!.ids as number[], args!.approve as boolean)
      case 'approvals_history': return history
      case 'approval_get': return onGet(args!.id as number)
      case 'approvals_export': return onExport()
      case 'log_export_reveal': return onReveal(args!.path as string)
      case 'accounts_list': return []
      case 'settings_get': return { idleMinutes: 30, appearance: 'system', showInDock: true }
      case 'log_list': return []
      default: return undefined
    }
  })
})
afterEach(() => {
  for (const wrapper of mounted.splice(0)) wrapper.unmount()
  vi.useRealTimers()
  store.approvals = []
  approvalState.history = []
  waiting = []
  history = []
  onDecide = async (id) => {
    waiting = waiting.filter((p) => p.id !== id)
  }
  onDecideAll = async (ids) => {
    waiting = waiting.filter((p) => !ids.includes(p.id))
    return { decided: ids, gone: [] }
  }
  onGet = async (id) => ({ ...history.find((r) => r.id === id)!, args: { omschrijving: 'Bankkosten' }, body: { Rekening: '1010' }, result: { MutatieNr: 812 } })
  onExport = async () => '/Users/jan/Downloads/eb-mcp-akkoord.jsonl.gz'
  onReveal = async () => {}
})

// The whole window, as in the app: the toolbar's Teleport target, the sidebar and the banner live in the shell.
async function mountAt(path: string) {
  await router.push(path)
  await router.isReady()
  const wrapper = mount(App, { global: { plugins: [router] }, attachTo: document.body })
  mounted.push(wrapper)
  await flushPromises()
  return wrapper
}

const buttons = (root: ParentNode, text: string) =>
  [...root.querySelectorAll<HTMLButtonElement>('button')].filter((b) => b.textContent?.trim() === text)
const alertDialog = () => document.querySelector<HTMLElement>('[role="alertdialog"]')
const decideCalls = () => invoke.mock.calls.filter((c) => c[0] === 'approval_decide' || c[0] === 'approvals_decide_all')

/** The queue's rows, the one of a proposal, the selected one, and the detail beside them. */
const rows = () => [...document.querySelectorAll<HTMLButtonElement>('.q-list .q-row')]
const row = (id: number) => document.querySelector<HTMLButtonElement>(`#q-row-${id}`)!
const rowNumbers = () => rows().map((r) => r.querySelector('.pill')?.textContent)
const selectedRows = () => rows().filter((r) => r.getAttribute('aria-current') === 'true')
const detail = () => document.querySelector<HTMLElement>('#q-detail')!
const detailTitle = () => detail().querySelector('#q-detail-title')?.textContent
const detailPosition = () => detail().querySelector('.q-pos')?.textContent

async function choose(id: number) {
  row(id).click()
  await flushPromises()
}

async function press(key: string) {
  document.querySelector('.q-list')!.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
  await flushPromises()
}

describe('the queue', () => {
  it('lists each waiting change oldest first, and shows the first one beside the list', async () => {
    waiting = [proposal(), proposal({ id: 8, account: 'Bakkerij Zuid', tool: 'relatie_wijzigen', title: 'Relatie wijzigen: Jansen', client: 'Cursor' })]
    await mountAt('/akkoord')
    expect(rowNumbers()).toEqual(['#7', '#8'])
    // What happens and its subject on two lines, from "what: subject"; account and age under them.
    expect(row(7).querySelector('.q-what')?.textContent).toBe('Mutatie toevoegen')
    expect(row(7).querySelector('.q-subject')?.textContent).toBe('Geld uitgegeven')
    expect(row(7).querySelector('.q-meta')?.textContent).toBe('Studio Noord · 5 min geleden')
    expect(row(8).querySelector('.q-meta')?.textContent).toContain('Bakkerij Zuid')
    // The account's initials are gone; the kind of change has an icon instead.
    expect(document.querySelector('.q-list .avatar')).toBeNull()
    expect(row(7).querySelector('.kind')?.classList.contains('accent')).toBe(true)
    expect(row(8).querySelector('.kind')?.classList.contains('neutral')).toBe(true)

    // The first is selected on open, and only the list's selected row is a tab stop.
    expect(selectedRows()).toEqual([row(7)])
    expect(rows().map((r) => r.tabIndex)).toEqual([0, -1])
    expect(detail().getAttribute('role')).toBe('region')
    expect(detail().getAttribute('aria-labelledby')).toBe('q-detail-title')
    expect(detail().querySelector('.q-of')?.textContent).toBe('#7Voorstel 7 van Claude')
    expect(detailTitle()).toBe('Mutatie toevoegen: Geld uitgegeven')
    expect(detail().querySelector('.q-asked')?.textContent).toBe('Studio Noord · gevraagd door Claude · 5 min geleden')
    expect(detail().textContent).toContain('Wat er verandert')
    expect([...detail().querySelectorAll('dt')].map((e) => e.textContent)).toEqual(['Rekening', 'Omschrijving'])
    // The value keeps its line break; the stylesheet shows it with white-space: pre-line.
    expect([...detail().querySelectorAll('dd')].map((e) => e.textContent)).toEqual(['1010 Bank', 'Regel een\nRegel twee'])
    expect(detailPosition()).toBe('Voorstel #7, 1 van 2 in de wachtrij')
    // The answer is given in the detail only, for the one shown there.
    expect(buttons(detail(), 'Goedkeuren')).toHaveLength(1)
    expect(buttons(detail(), 'Weigeren')).toHaveLength(1)
    expect(buttons(document.querySelector('.q-list')!, 'Goedkeuren')).toHaveLength(0)
  })

  it('shows the proposal whose row is clicked', async () => {
    waiting = [proposal(), proposal({ id: 8, account: 'Bakkerij Zuid', title: 'Relatie wijzigen: Jansen', client: 'Cursor', lines: [{ label: 'Relatie', value: 'Jansen' }] })]
    await mountAt('/akkoord')
    await choose(8)
    expect(selectedRows()).toEqual([row(8)])
    expect(rows().map((r) => r.tabIndex)).toEqual([-1, 0])
    expect(detailTitle()).toBe('Relatie wijzigen: Jansen')
    expect(detail().querySelector('.q-of')?.textContent).toBe('#8Voorstel 8 van Cursor')
    expect([...detail().querySelectorAll('dt')].map((e) => e.textContent)).toEqual(['Relatie'])
    expect(detailPosition()).toBe('Voorstel #8, 2 van 2 in de wachtrij')
  })

  it('moves the selection with the arrow keys, Home and End', async () => {
    waiting = [proposal(), proposal({ id: 8 }), proposal({ id: 9 })]
    await mountAt('/akkoord')
    row(7).focus()
    await press('ArrowDown')
    expect(selectedRows()).toEqual([row(8)])
    expect(document.activeElement).toBe(row(8))
    expect(detailPosition()).toBe('Voorstel #8, 2 van 3 in de wachtrij')
    await press('ArrowDown')
    await press('ArrowDown')
    // It stops at the end rather than wrap.
    expect(selectedRows()).toEqual([row(9)])
    await press('ArrowUp')
    expect(selectedRows()).toEqual([row(8)])
    expect(document.activeElement).toBe(row(8))
    await press('Home')
    expect(selectedRows()).toEqual([row(7)])
    await press('End')
    expect(selectedRows()).toEqual([row(9)])
    expect(rows().map((r) => r.tabIndex)).toEqual([-1, -1, 0])
    // Other keys are left alone.
    await press('a')
    expect(selectedRows()).toEqual([row(9)])
  })

  it('shows the shared empty state when nothing waits', async () => {
    await mountAt('/akkoord')
    expect(document.querySelector('.empty h2')?.textContent).toBe('Niets wacht op je akkoord')
    expect(buttons(document, 'Alles weigeren')).toHaveLength(0)
    expect(document.querySelector('#q-detail')).toBeNull()
  })

  it('sends a change only on Goedkeuren, then shows the next one', async () => {
    waiting = [proposal(), proposal({ id: 8, title: 'Factuur F00012 verwijderen' })]
    await mountAt('/akkoord')
    // Picking a row approves nothing.
    await choose(8)
    await choose(7)
    expect(decideCalls()).toEqual([])
    buttons(detail(), 'Goedkeuren')[0]!.click()
    await flushPromises()
    expect(decideCalls()).toEqual([['approval_decide', { id: 7, approve: true }]])
    expect(rowNumbers()).toEqual(['#8'])
    expect(selectedRows()).toEqual([row(8)])
    expect(detailTitle()).toBe('Factuur F00012 verwijderen')
    expect(detailPosition()).toBe('Voorstel #8, 1 van 1 in de wachtrij')
  })

  it('refuses on Weigeren and never approves by accident', async () => {
    waiting = [proposal()]
    await mountAt('/akkoord')
    const refuse = deferred<void>()
    onDecide = () => refuse.promise
    const button = buttons(detail(), 'Weigeren')[0]!
    button.click()
    await flushPromises()
    // Busy while the app answers, and nothing else can be clicked meanwhile.
    expect(button.getAttribute('aria-busy')).toBe('true')
    expect(buttons(detail(), 'Goedkeuren')[0]!.disabled).toBe(true)
    waiting = []
    refuse.resolve()
    await flushPromises()
    expect(decideCalls()).toEqual([['approval_decide', { id: 7, approve: false }]])
    expect(invoke).not.toHaveBeenCalledWith('approval_decide', expect.objectContaining({ approve: true }))
    expect(document.querySelector('.empty h2')?.textContent).toBe('Niets wacht op je akkoord')
  })

  it('selects the one before after the last one is answered', async () => {
    waiting = [proposal(), proposal({ id: 8 }), proposal({ id: 9, title: 'Relatie toevoegen: Jansen' })]
    await mountAt('/akkoord')
    await choose(9)
    buttons(detail(), 'Weigeren')[0]!.click()
    await flushPromises()
    expect(decideCalls()).toEqual([['approval_decide', { id: 9, approve: false }]])
    expect(rowNumbers()).toEqual(['#7', '#8'])
    expect(selectedRows()).toEqual([row(8)])
    expect(detailPosition()).toBe('Voorstel #8, 2 van 2 in de wachtrij')
  })

  it('moves the selection on when the selected one is answered elsewhere', async () => {
    waiting = [proposal(), proposal({ id: 8 }), proposal({ id: 9 })]
    await mountAt('/akkoord')
    await choose(8)
    // Answered in another window, or lapsed: the app sends a fresh queue.
    waiting = [proposal(), proposal({ id: 9 })]
    await refreshApprovals()
    await flushPromises()
    expect(selectedRows()).toEqual([row(9)])
    expect(detailPosition()).toBe('Voorstel #9, 2 van 2 in de wachtrij')
    waiting = [proposal()]
    await refreshApprovals()
    await flushPromises()
    expect(selectedRows()).toEqual([row(7)])
    // One that arrives meanwhile leaves the selection where it is.
    waiting = [proposal(), proposal({ id: 10 })]
    await refreshApprovals()
    await flushPromises()
    expect(selectedRows()).toEqual([row(7)])
    expect(decideCalls()).toEqual([])
  })

  it('says so when the proposal was already gone', async () => {
    waiting = [proposal()]
    await mountAt('/akkoord')
    onDecide = async () => {
      waiting = []
      throw 'Dit voorstel wacht niet meer: het is al beantwoord of verlopen.'
    }
    buttons(detail(), 'Goedkeuren')[0]!.click()
    await flushPromises()
    // The row has left; the reason stays above the list.
    expect(rows()).toHaveLength(0)
    expect(document.querySelector('.feedback[role="alert"]')?.textContent).toContain('wacht niet meer')
  })

  it('marks a change that cannot be undone and gives it the destructive button', async () => {
    waiting = [proposal(), proposal({ id: 8, tool: 'factuur_verwijderen', title: 'Factuur verwijderen: F00012', danger: true })]
    await mountAt('/akkoord')
    expect(row(8).querySelector('.danger-flag')?.textContent).toBe('Niet terug te draaien')
    expect(row(8).querySelector('.kind')?.classList.contains('danger')).toBe(true)
    expect(row(7).querySelector('.danger-flag')).toBeNull()
    expect(row(7).querySelector('.kind')?.classList.contains('danger')).toBe(false)
    // No red edge on the rows any more.
    expect(rows().some((r) => r.classList.contains('danger'))).toBe(false)

    expect(detail().querySelector('.danger-note')).toBeNull()
    const plain = buttons(detail(), 'Goedkeuren')[0]!
    expect(plain.classList.contains('danger-fill')).toBe(false)
    // One filled button per screen: the detail's Goedkeuren, not Alles goedkeuren.
    expect(plain.classList.contains('primary')).toBe(true)
    const filled = () => [...document.querySelectorAll('.page button.primary, .page button.danger-fill')]
    expect(filled()).toEqual([plain])
    expect(buttons(document, 'Alles goedkeuren (2)')[0]!.classList.contains('primary')).toBe(false)

    await choose(8)
    expect(detail().querySelector('.danger-note')?.textContent).toBe('Dit is niet terug te draaien vanuit EB MCP.')
    const destructive = buttons(detail(), 'Goedkeuren')[0]!
    expect(destructive.classList.contains('danger-fill')).toBe(true)
    expect(destructive.classList.contains('primary')).toBe(false)
    expect(filled()).toEqual([destructive])
  })

  it('splits a title into what happens and its subject', () => {
    expect(splitTitle('Relatie verwijderen: EB MCP proef wachtrij')).toEqual({ what: 'Relatie verwijderen', subject: 'EB MCP proef wachtrij' })
    // Only the first ": " splits; the subject keeps the rest.
    expect(splitTitle('Mutatie toevoegen: Bankkosten: maart')).toEqual({ what: 'Mutatie toevoegen', subject: 'Bankkosten: maart' })
    expect(splitTitle('Factuur F00012 verwijderen')).toEqual({ what: 'Factuur F00012 verwijderen', subject: null })
    expect(splitTitle('Tijd 10:30')).toEqual({ what: 'Tijd 10:30', subject: null })
    expect(splitTitle('Leeg: ')).toEqual({ what: 'Leeg: ', subject: null })
  })

  it('shows a title without a subject on one line', async () => {
    waiting = [proposal({ title: 'Abonnementen factureren', tool: 'abonnementen_factureren' })]
    await mountAt('/akkoord')
    expect(row(7).querySelector('.q-what')?.textContent).toBe('Abonnementen factureren')
    expect(row(7).querySelector('.q-subject')).toBeNull()
  })

  it('picks an icon for the kind of change from the tool name', () => {
    const kind = (tool: string, danger = false) => proposalKind({ tool, danger })
    expect(kind('mutatie_toevoegen')).toEqual({ icon: 'plus', tone: 'accent' })
    expect(kind('archief_uploaden')).toEqual({ icon: 'plus', tone: 'accent' })
    expect(kind('relatie_wijzigen')).toEqual({ icon: 'pencil', tone: 'neutral' })
    expect(kind('archief_koppelen')).toEqual({ icon: 'pencil', tone: 'neutral' })
    expect(kind('relatie_verwijderen')).toEqual({ icon: 'trash', tone: 'neutral' })
    expect(kind('factuur_mailen')).toEqual({ icon: 'mail', tone: 'neutral' })
    expect(kind('factuur_crediteren')).toEqual({ icon: 'file-text', tone: 'neutral' })
    expect(kind('abonnementen_factureren')).toEqual({ icon: 'file-text', tone: 'neutral' })
    expect(kind('offerte_mailen')).toEqual({ icon: 'mail', tone: 'neutral' })
    expect(kind('offerte_factureren')).toEqual({ icon: 'file-text', tone: 'neutral' })
    expect(kind('archief_ontkoppelen')).toEqual({ icon: 'pencil', tone: 'neutral' })
    expect(kind('iets_anders')).toEqual({ icon: 'square-dot', tone: 'neutral' })
    // A change that cannot be undone keeps its icon on the red tint.
    expect(kind('relatie_verwijderen', true)).toEqual({ icon: 'trash', tone: 'danger' })
    expect(kind('mutatie_toevoegen', true)).toEqual({ icon: 'plus', tone: 'danger' })
  })

  it('follows the selection through a changed queue as a pure function', () => {
    expect(followSelection([], [], null)).toBeNull()
    expect(followSelection([], [7, 8], null)).toBe(7)
    expect(followSelection([7, 8, 9], [7, 8, 9], 8)).toBe(8)
    // Gone: the next one takes its place, or the one before when it was the last.
    expect(followSelection([7, 8, 9], [7, 9], 8)).toBe(9)
    expect(followSelection([7, 8, 9], [7, 8], 9)).toBe(8)
    expect(followSelection([7, 8, 9], [9], 7)).toBe(9)
    // Several gone at once: the first that still waits after it.
    expect(followSelection([7, 8, 9, 10], [7, 10], 8)).toBe(10)
    // The last old one gone while a new one came in: the new one is next in the list.
    expect(followSelection([7, 8], [7, 9], 8)).toBe(9)
    expect(followSelection([7, 8], [], 8)).toBeNull()
    // Not in the list before (such as a ?open= id that no longer waits): the first.
    expect(followSelection([7, 8], [7, 8], 3)).toBe(7)
  })
})

describe('Alles goedkeuren', () => {
  it('asks first, then approves exactly the changes it named, and reports the ones already answered', async () => {
    waiting = [
      proposal(),
      proposal({ id: 8, account: 'Bakkerij Zuid', title: 'Factuur F00012 verwijderen', danger: true }),
      proposal({ id: 9, title: 'Relatie toevoegen: Jansen' }),
    ]
    await mountAt('/akkoord')
    buttons(document, 'Alles goedkeuren (3)')[0]!.click()
    await flushPromises()
    expect(decideCalls()).toEqual([])
    const dialog = alertDialog()!
    expect(dialog.getAttribute('aria-label')).toBe('3 wijzigingen goedkeuren?')
    expect(dialog.textContent).toContain('Voor Studio Noord en Bakkerij Zuid.')
    expect(dialog.textContent).toContain('1 daarvan is niet terug te draaien.')
    expect(buttons(dialog, 'Goedkeuren')[0]!.classList.contains('danger-fill')).toBe(true)
    // It names what it approves, so nothing is approved unread.
    const named = [...dialog.querySelectorAll('.confirm-list li')]
    expect(named.map((li) => li.textContent)).toEqual([
      '7 · Mutatie toevoegen: Geld uitgegeven · Studio Noord', '8 · Factuur F00012 verwijderen · Bakkerij Zuid', '9 · Relatie toevoegen: Jansen · Studio Noord',
    ])
    expect(named.map((li) => li.classList.contains('danger'))).toEqual([false, true, false])

    // One comes in while the question is open: it is not part of what the user confirms.
    waiting = [...waiting, proposal({ id: 10, title: 'Mutatie toevoegen: Nieuw' })]
    await refreshApprovals()
    await flushPromises()
    expect(alertDialog()!.textContent).toContain('Intussen kwam er 1 wijziging bij; die blijft wachten.')

    onDecideAll = async (ids) => {
      waiting = waiting.filter((p) => !ids.includes(p.id))
      return { decided: [7, 9], gone: [8] }
    }
    buttons(alertDialog()!, 'Goedkeuren')[0]!.click()
    await flushPromises()
    expect(decideCalls()).toEqual([['approvals_decide_all', { ids: [7, 8, 9], approve: true }]])
    expect(alertDialog()).toBeNull()
    const note = document.querySelector('.feedback[role="status"]')!
    expect(note.textContent).toContain('2 wijzigingen goedgekeurd')
    expect(note.textContent).toContain('1 wijziging was al beantwoord of vervallen')
    expect(rowNumbers()).toEqual(['#10'])
    expect(detailTitle()).toBe('Mutatie toevoegen: Nieuw')
  })

  it('names at most eight changes and counts the rest', async () => {
    waiting = Array.from({ length: 11 }, (_, i) => proposal({ id: 20 + i, title: `Mutatie ${i + 1}` }))
    await mountAt('/akkoord')
    buttons(document, 'Alles goedkeuren (11)')[0]!.click()
    await flushPromises()
    const named = [...alertDialog()!.querySelectorAll('.confirm-list li')].map((li) => li.textContent)
    expect(named).toHaveLength(9)
    expect(named.at(-1)).toBe('en nog 3')
  })

  it('warns before approving for an account that is not signed in', async () => {
    waiting = [proposal(), proposal({ id: 8, accountId: 'a2', account: 'Bakkerij Zuid' })]
    await mountAt('/akkoord')
    store.accounts = [
      { id: 'a1', label: 'Studio Noord', session: null },
      { id: 'a2', label: 'Bakkerij Zuid', session: { accountId: 'a2', label: 'Bakkerij Zuid', administration: null, startedAt: 1, expired: false } },
    ] as unknown as AccountView[]
    store.loaded = true
    await flushPromises()
    expect(rows().map((r) => r.querySelector('.signed-out-flag')?.textContent ?? null)).toEqual(['Niet ingelogd', null])
    // The first is selected: its detail says what happens on Goedkeuren; the other's does not.
    expect(detail().querySelector('.signed-out')?.textContent).toContain('Studio Noord is niet ingelogd in EB MCP')
    await choose(8)
    expect(detail().querySelector('.signed-out')).toBeNull()
    buttons(document, 'Alles goedkeuren (2)')[0]!.click()
    await flushPromises()
    expect(alertDialog()!.textContent).toContain('Studio Noord is niet ingelogd in EB MCP: die wijzigingen mislukken tot je daar weer inlogt.')
    store.accounts = []
  })

  it('approves nothing when the question is cancelled', async () => {
    waiting = [proposal()]
    await mountAt('/akkoord')
    buttons(document, 'Alles goedkeuren (1)')[0]!.click()
    await flushPromises()
    expect(alertDialog()!.getAttribute('aria-label')).toBe('1 wijziging goedkeuren?')
    buttons(alertDialog()!, 'Annuleren')[0]!.click()
    await flushPromises()
    expect(alertDialog()).toBeNull()
    expect(decideCalls()).toEqual([])
  })

  it('refuses all with Alles weigeren, without approving any', async () => {
    waiting = [proposal(), proposal({ id: 8 })]
    await mountAt('/akkoord')
    buttons(document, 'Alles weigeren')[0]!.click()
    await flushPromises()
    expect(decideCalls()).toEqual([['approvals_decide_all', { ids: [7, 8], approve: false }]])
    expect(document.querySelector('.feedback')?.textContent).toContain('2 wijzigingen geweigerd.')
  })
})

describe('the history', () => {
  beforeEach(() => {
    history = [
      record({ id: 12, title: 'Factuur F00012 verwijderen', account: 'Bakkerij Zuid', status: 'failed', decision: 'approved', decidedAt: 2_000, error: 'e-Boekhouden: factuur is al betaald' }),
      record({ id: 11, title: 'Relatie toevoegen: Jansen', status: 'done', decision: 'auto', decidedAt: 1_500 }),
      record({ id: 10, title: 'Mutatie toevoegen: Bankkosten', status: 'refused', decision: 'refused', decidedAt: 1_200 }),
      record({ id: 9, status: 'lapsed', decision: 'lapsed', decidedAt: 1_100 }),
    ]
  })

  async function openHistory() {
    const wrapper = await mountAt('/akkoord')
    document.querySelector<HTMLButtonElement>('#tab-history')!.click()
    await flushPromises()
    return wrapper
  }
  const titles = () => [...document.querySelectorAll('.record .r-title')].map((e) => e.textContent)

  it('lists every record newest first with its status, and marks the ones made without approval', async () => {
    await openHistory()
    expect(titles()).toEqual(['Factuur F00012 verwijderen', 'Relatie toevoegen: Jansen', 'Mutatie toevoegen: Bankkosten', 'Mutatie toevoegen: Geld uitgegeven'])
    expect([...document.querySelectorAll('.record .chip.status')].map((e) => e.textContent)).toEqual(['Mislukt', 'Uitgevoerd', 'Geweigerd', 'Vervallen'])
    expect([...document.querySelectorAll('.record')].map((r) => r.querySelector('.chip.auto')?.textContent ?? null)).toEqual([null, 'Zonder akkoord', null, null])
  })

  it('filters by status and searches the title and account', async () => {
    await openHistory()
    const select = document.querySelector<HTMLSelectElement>('select[aria-label="Status"]')!
    expect([...select.options].map((o) => o.textContent)).toEqual(['Alle statussen', 'Wacht', 'Bezig', 'Uitgevoerd', 'Mislukt', 'Geweigerd', 'Vervallen', 'Ingetrokken'])
    select.value = 'refused'
    select.dispatchEvent(new Event('change'))
    await flushPromises()
    expect(titles()).toEqual(['Mutatie toevoegen: Bankkosten'])

    select.value = ''
    select.dispatchEvent(new Event('change'))
    const search = document.querySelector<HTMLInputElement>('input[type="search"]')!
    search.value = 'bakkerij'
    search.dispatchEvent(new Event('input'))
    await flushPromises()
    expect(titles()).toEqual(['Factuur F00012 verwijderen'])

    search.value = 'mutatie geld'
    search.dispatchEvent(new Event('input'))
    await flushPromises()
    expect(titles()).toEqual(['Mutatie toevoegen: Geld uitgegeven'])

    select.value = 'done'
    select.dispatchEvent(new Event('change'))
    await flushPromises()
    expect(titles()).toEqual([])
    expect(document.querySelector('.empty h2')?.textContent).toBe('Geen wijzigingen voor deze filters')
    buttons(document, 'Filters wissen')[0]!.click()
    await flushPromises()
    expect(titles()).toHaveLength(4)
  })

  it('opens a detail with the human view and loads the technical data for it', async () => {
    await openHistory()
    const get = deferred<ApprovalDetail>()
    onGet = () => get.promise
    document.querySelector<HTMLButtonElement>('.record')!.click()
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('approval_get', { id: 12 })
    const detail = document.querySelector<HTMLElement>('aside.detail')!
    expect(detail.querySelector('.detail-title')?.textContent).toBe('Factuur F00012 verwijderen')
    expect(detail.querySelector('.outcome')?.textContent).toContain('Mislukt: e-Boekhouden: factuur is al betaald')
    const facts = [...detail.querySelectorAll('.kv dt')].map((dt) => [dt.textContent, dt.nextElementSibling?.textContent])
    expect(facts).toEqual([
      ['Voorstel', '12'], ['Gevraagd door', 'Claude'], ['Account', 'Bakkerij Zuid'], ['Gevraagd', expect.any(String)], ['Beantwoord', expect.any(String)],
      ['Antwoord', 'Goedgekeurd'], ['App-versie', '0.4.0'],
    ])
    expect([...detail.querySelectorAll('dl.lines dt')].map((e) => e.textContent)).toEqual(['Rekening', 'Omschrijving'])

    buttons(detail, 'Technisch')[0]!.click()
    await flushPromises()
    expect(detail.querySelector('.loading')?.textContent).toContain('laden')
    get.resolve({ ...history[0]!, args: { factuur: 'F00012' }, body: { Id: 4411 }, result: { ok: false } })
    await flushPromises()
    const json = [...detail.querySelectorAll('pre.json')].map((e) => e.textContent)
    expect(json).toHaveLength(3)
    expect(json[0]).toContain('"factuur": "F00012"')
    expect(json[1]).toContain('"Id": 4411')
    expect(json[2]).toContain('"ok": false')

    const writeText = vi.fn(async (_text: string) => {})
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    detail.querySelector<HTMLButtonElement>('button.copy')!.click()
    await flushPromises()
    expect(JSON.parse(writeText.mock.calls[0]![0])).toEqual({ args: { factuur: 'F00012' }, body: { Id: 4411 }, result: { ok: false } })
  })

  it('says what happened to a change that was refused, lapsed or cut off', async () => {
    history = [
      record({ id: 15, status: 'lapsed', decision: 'approved', decidedAt: 3_000, error: 'EB MCP is gestopt terwijl deze wijziging liep. Kijk in e-Boekhouden wat er al is doorgevoerd.' }),
      record({ id: 14, status: 'lapsed', decision: 'lapsed', decidedAt: 2_000, error: 'EB MCP is opnieuw gestart voordat de gebruiker antwoordde; het voorstel is vervallen. Er is niets gewijzigd.' }),
      record({ id: 13, status: 'refused', decision: 'refused', decidedAt: 1_000 }),
      // Taken back by the AI client, with and without the reason the app keeps.
      record({ id: 12, status: 'withdrawn', decision: 'withdrawn', decidedAt: 900, error: 'Ingetrokken door Claude: verkeerd bedrag. Er is niets gewijzigd.' }),
      record({ id: 11, status: 'withdrawn', decision: 'withdrawn', decidedAt: 800 }),
    ]
    await openHistory()
    const outcomeOf = async (index: number) => {
      document.querySelectorAll<HTMLButtonElement>('.record')[index]!.click()
      await flushPromises()
      const note = document.querySelector<HTMLElement>('aside.detail .outcome')!
      return [note.classList.contains('bad'), note.textContent?.trim()]
    }
    expect(await outcomeOf(0)).toEqual([true, 'EB MCP is gestopt terwijl deze wijziging liep. Kijk in e-Boekhouden wat er al is doorgevoerd.'])
    expect(await outcomeOf(1)).toEqual([false, 'EB MCP is opnieuw gestart voordat de gebruiker antwoordde; het voorstel is vervallen. Er is niets gewijzigd.'])
    expect(await outcomeOf(2)).toEqual([false, 'Geweigerd. Er is niets gewijzigd in e-Boekhouden.'])
    expect(await outcomeOf(3)).toEqual([false, 'Ingetrokken door Claude: verkeerd bedrag. Er is niets gewijzigd.'])
    expect(await outcomeOf(4)).toEqual([false, 'Ingetrokken door de AI-client. Er is niets gewijzigd in e-Boekhouden.'])
    // The list names it too, and it can be filtered on.
    expect(statusOf('withdrawn')).toMatchObject({ label: 'Ingetrokken', tone: 'off' })
    expect(decisionLabel('withdrawn')).toBe('Ingetrokken door de AI-client')
  })

  it('names an answer given to all waiting changes at once', async () => {
    history = [
      record({ id: 21, title: 'Samen goedgekeurd', status: 'done', decision: 'approved', decidedAt: 2_000, bulk: true }),
      record({ id: 20, title: 'Samen geweigerd', status: 'refused', decision: 'refused', decidedAt: 1_000, bulk: true }),
    ]
    await openHistory()
    const answerOf = async (index: number) => {
      document.querySelectorAll<HTMLButtonElement>('.record')[index]!.click()
      await flushPromises()
      const dt = [...document.querySelectorAll('aside.detail .kv dt')].find((e) => e.textContent === 'Antwoord')!
      return dt.nextElementSibling?.textContent
    }
    expect(await answerOf(0)).toBe('Goedgekeurd · Alles goedkeuren')
    expect(await answerOf(1)).toBe('Geweigerd · Alles weigeren')
  })

  it('opens a record that no longer waits in Geschiedenis with its detail, from ?open=', async () => {
    await mountAt('/akkoord?open=11')
    expect(document.querySelector('#tab-history')?.getAttribute('aria-selected')).toBe('true')
    expect(document.querySelector('.record.sel .r-title')?.textContent).toBe('Relatie toevoegen: Jansen')
    expect(document.querySelector('aside.detail .detail-title')?.textContent).toBe('Relatie toevoegen: Jansen')
    expect(invoke).toHaveBeenCalledWith('approval_get', { id: 11 })
    expect(rows()).toHaveLength(0)
  })

  it('opens a record that still waits in the queue, from ?open=', async () => {
    // Not the first: ?open= wins over the first-one default.
    waiting = [proposal({ id: 8, title: 'Relatie toevoegen: Jansen' }), proposal({ id: 9 })]
    await mountAt('/akkoord?open=9')
    expect(document.querySelector('#tab-queue')?.getAttribute('aria-selected')).toBe('true')
    expect(selectedRows()).toEqual([row(9)])
    expect(detailTitle()).toBe('Mutatie toevoegen: Geld uitgegeven')
    expect(detailPosition()).toBe('Voorstel #9, 2 van 2 in de wachtrij')
  })

  it('shows the reason when the technical data cannot be loaded', async () => {
    await openHistory()
    onGet = async () => {
      throw 'Dit voorstel bestaat niet meer.'
    }
    document.querySelector<HTMLButtonElement>('.record')!.click()
    await flushPromises()
    buttons(document.querySelector('aside.detail')!, 'Technisch')[0]!.click()
    await flushPromises()
    expect(document.querySelector('aside.detail [role="alert"]')?.textContent).toBe('Dit voorstel bestaat niet meer.')
  })

  it('filters and searches as a pure function too', () => {
    expect(filterHistory(history, 'done', '').map((r) => r.id)).toEqual([11])
    expect(filterHistory(history, '', 'JANSEN').map((r) => r.id)).toEqual([11])
    expect(filterHistory(history, '', '').map((r) => r.id)).toEqual([12, 11, 10, 9])
    expect(nameList(['A', 'B', 'A', 'C'])).toBe('A, B en C')
    expect(ago(Date.now() - 10_000)).toBe('zojuist')
  })
})

describe('Exporteer on Akkoord', () => {
  const exportButton = () => document.querySelector<HTMLButtonElement>('#toolbar-actions button.export')!

  it('writes the history and shows where, with Toon in Finder', async () => {
    const run = deferred<string>()
    onExport = () => run.promise
    await mountAt('/akkoord')
    exportButton().click()
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('approvals_export')
    expect(exportButton().getAttribute('aria-busy')).toBe('true')
    run.resolve('/Users/jan/Downloads/eb-mcp-akkoord.jsonl.gz')
    await flushPromises()
    expect(exportButton().getAttribute('aria-busy')).toBe('false')
    const note = document.querySelector('.export-note[role="status"]')!
    expect(note.querySelector('.export-path')?.textContent).toBe('/Users/jan/Downloads/eb-mcp-akkoord.jsonl.gz')
    const reveal = note.querySelector<HTMLButtonElement>('button.reveal')!
    expect(reveal.textContent).toBe('Toon in Finder')
    reveal.click()
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('log_export_reveal', { path: '/Users/jan/Downloads/eb-mcp-akkoord.jsonl.gz' })
  })

  it('shows the reason when the export fails', async () => {
    onExport = async () => {
      throw 'De export kon niet worden opgeslagen: de schijf is vol.'
    }
    await mountAt('/akkoord')
    exportButton().click()
    await flushPromises()
    expect(document.querySelector('.export-note[role="alert"]')?.textContent).toContain('de schijf is vol')
    expect(document.querySelector('.export-path')).toBeNull()
  })
})

describe('signs that something waits', () => {
  const badge = () => document.querySelector('a[href="#/akkoord"] .nav-badge')
  const banner = () => document.querySelector('.approval-notice')

  it('puts Akkoord between Accounts and Verzoeken with a count, and a banner on other pages', async () => {
    waiting = [proposal(), proposal({ id: 8 })]
    await mountAt('/accounts')
    expect([...document.querySelectorAll('nav a')].map((a) => a.querySelector('span')?.textContent)).toEqual(['Dashboard', 'Accounts', 'Akkoord', 'Verzoeken', 'Koppelen', 'Instellingen'])
    expect(badge()?.textContent).toBe('2')
    expect(document.querySelector('a[href="#/akkoord"]')?.getAttribute('aria-label')).toBe('Akkoord, 2 wijzigingen wachten op je akkoord')
    expect(banner()?.textContent).toContain('2 wijzigingen wachten op je akkoord')

    waiting = [proposal()]
    await refreshApprovals()
    await flushPromises()
    expect(badge()?.textContent).toBe('1')
    expect(banner()?.textContent).toContain('1 wijziging wacht op je akkoord')

    buttons(banner()!, 'Bekijken')[0]!.click()
    await flushPromises()
    expect(router.currentRoute.value.path).toBe('/akkoord')
    // Not on Akkoord itself: the page shows the queue.
    expect(banner()).toBeNull()
    expect(badge()?.textContent).toBe('1')

    waiting = []
    await refreshApprovals()
    await flushPromises()
    expect(badge()).toBeNull()
  })

  it('shows no badge and no banner while nothing waits', async () => {
    await mountAt('/accounts')
    expect(badge()).toBeNull()
    expect(banner()).toBeNull()
  })
})

describe('the toast for a new change', () => {
  const toasts = () => [...document.querySelectorAll<HTMLElement>('.toasts .toast')]

  it('appears for a new proposal on another page, not for the ones present at load, and leaves by itself', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] })
    waiting = [proposal()]
    await mountAt('/accounts')
    expect(toasts()).toHaveLength(0)
    expect(document.querySelector('.toasts')?.getAttribute('aria-live')).toBe('polite')

    waiting = [proposal(), proposal({ id: 8, title: 'Factuur F00012 verwijderen', account: 'Bakkerij Zuid' })]
    await refreshApprovals()
    await flushPromises()
    expect(toasts()).toHaveLength(1)
    expect(toasts()[0]!.querySelector('.toast-title')?.textContent).toBe('Factuur F00012 verwijderen')
    expect(toasts()[0]!.querySelector('.toast-text')?.textContent).toContain('Bakkerij Zuid')
    expect(toasts()[0]!.querySelector('.toast-text')?.textContent).toContain('Voorstel ')
    expect(buttons(toasts()[0]!, 'Bekijken')).toHaveLength(1)

    // Hovering stops the clock; leaving starts it again with the time that was left.
    vi.advanceTimersByTime(5_000)
    document.querySelector('.toasts')!.dispatchEvent(new MouseEvent('mouseenter'))
    await flushPromises()
    vi.advanceTimersByTime(20_000)
    await flushPromises()
    expect(toasts()).toHaveLength(1)
    document.querySelector('.toasts')!.dispatchEvent(new MouseEvent('mouseleave'))
    await flushPromises()
    vi.advanceTimersByTime(2_900)
    await flushPromises()
    expect(toasts()).toHaveLength(1)
    vi.advanceTimersByTime(200)
    await flushPromises()
    expect(toasts()).toHaveLength(0)
  })

  it('stacks at most three, then says how many more, and closes with the cross', async () => {
    await mountAt('/accounts')
    waiting = [1, 2, 3, 4, 5].map((id) => proposal({ id, title: `Wijziging ${id}` }))
    await refreshApprovals()
    await flushPromises()
    // Newest on top.
    expect(toasts().map((t) => t.querySelector('.toast-title')?.textContent)).toEqual(['Wijziging 5', 'Wijziging 4', 'Wijziging 3'])
    expect(document.querySelector('.toasts .more')?.textContent).toBe('en 2 meer')
    toasts()[0]!.querySelector<HTMLButtonElement>('button[aria-label="Melding sluiten"]')!.click()
    await flushPromises()
    expect(toasts().map((t) => t.querySelector('.toast-title')?.textContent)).toEqual(['Wijziging 4', 'Wijziging 3', 'Wijziging 2'])
    expect(document.querySelector('.toasts .more')?.textContent).toBe('en 1 meer')
  })

  it('opens the proposal on Akkoord with Bekijken, and leaves when it is answered elsewhere', async () => {
    await mountAt('/accounts')
    waiting = [proposal({ id: 8, title: 'Factuur F00012 verwijderen' }), proposal({ id: 9, title: 'Relatie toevoegen: Jansen' })]
    await refreshApprovals()
    await flushPromises()
    expect(toasts()).toHaveLength(2)

    waiting = [proposal({ id: 8, title: 'Factuur F00012 verwijderen' })]
    await refreshApprovals()
    await flushPromises()
    expect(toasts().map((t) => t.querySelector('.toast-title')?.textContent)).toEqual(['Factuur F00012 verwijderen'])

    buttons(toasts()[0]!, 'Bekijken')[0]!.click()
    await flushPromises()
    expect(router.currentRoute.value.fullPath).toBe('/akkoord?open=8')
    expect(toasts()).toHaveLength(0)
    expect(selectedRows()).toEqual([row(8)])
    expect(detailTitle()).toBe('Factuur F00012 verwijderen')
  })

  it('stays away on Akkoord itself', async () => {
    await mountAt('/akkoord')
    waiting = [proposal({ id: 8 })]
    await refreshApprovals()
    await flushPromises()
    expect(toasts()).toHaveLength(0)
    expect(rows()).toHaveLength(1)
  })
})

describe('stopping EB MCP', () => {
  it('says what is lost: running calls fail and waiting changes lapse', () => {
    expect(quitWarning(1, 0)).toBe('Er loopt nog 1 aanroep van een AI-client; die krijgt dan een fout. EB MCP vergeet alle sessies.')
    expect(quitWarning(0, 2)).toBe('2 wijzigingen wachten op je akkoord; die vervallen dan en worden niet uitgevoerd. EB MCP vergeet alle sessies.')
    expect(quitWarning(3, 1)).toBe(
      'Er lopen nog 3 aanroepen van een AI-client; die krijgen dan een fout. 1 wijziging wacht op je akkoord; die vervalt dan en wordt niet uitgevoerd. EB MCP vergeet alle sessies.',
    )
  })
})

describe('the request log', () => {
  it('names a call whose change waits for approval', () => {
    expect(statusLabel({ kind: 'mcp', outcome: 'waiting' } as McpEntry)).toBe('Wacht op akkoord')
  })
})

describe('the menu-bar panel', () => {
  it('opens the main window on Akkoord from the waiting item and from its own item', async () => {
    waiting = [proposal()]
    await mountAt('/panel')
    const item = document.querySelector<HTMLButtonElement>('button.item.waiting')!
    expect(item.textContent).toContain('1 wijziging wacht op je akkoord')
    item.click()
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('open_main', { route: '/akkoord' })

    invoke.mockClear()
    const own = [...document.querySelectorAll<HTMLButtonElement>('button.item')].find((b) => b.textContent?.trim() === 'Akkoord')!
    own.click()
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('open_main', { route: '/akkoord' })
  })
})

describe('the write setting of an account', () => {
  const account: AccountView = {
    id: 'a1', label: 'Studio Noord', username: 'jan@studionoord.example', store: 'system', opRef: null, opTitle: null, opVault: null,
    administration: 'Studio Noord B.V.', rights: { change: 'off', delete: 'off', send: 'off', heavy: 'off' }, session: null, signingIn: false, signin: null,
  }

  const ask = { change: 'ask', delete: 'ask', send: 'ask', heavy: 'ask' } as const
  const levels = (kind: string) => {
    const group = document.querySelector(`[role="radiogroup"][aria-label="${kind}"]`)!
    return [...group.querySelectorAll<HTMLButtonElement>('[role="radio"]')]
  }
  const checked = (kind: string) => levels(kind).find((o) => o.getAttribute('aria-checked') === 'true')?.textContent

  it('is read-only until the user chooses otherwise, and saves the choice', async () => {
    const wrapper = mount(AccountForm, { props: { account }, attachTo: document.body })
    mounted.push(wrapper)
    await flushPromises()
    const block = document.querySelector('.rights')!.parentElement!
    const presets = levels('AI-clients mogen in dit account')
    expect(presets.map((o) => o.textContent)).toEqual(['Alleen lezen', 'Alles met akkoord', 'Vrij boeken'])
    expect(presets.map((o) => o.getAttribute('aria-checked'))).toEqual(['true', 'false', 'false'])
    expect(block.textContent).toContain('niets wijzigen')
    expect(['Boeken en wijzigen', 'Verwijderen', 'Versturen', 'Ingrijpend'].map(checked)).toEqual(['Uit', 'Uit', 'Uit', 'Uit'])
    presets[1]!.click()
    await flushPromises()
    expect(presets[1]!.getAttribute('aria-checked')).toBe('true')
    expect(block.textContent).toContain('pas na Goedkeuren')
    expect(['Boeken en wijzigen', 'Verwijderen', 'Versturen', 'Ingrijpend'].map(checked)).toEqual(['Met akkoord', 'Met akkoord', 'Met akkoord', 'Met akkoord'])
    invoke.mockImplementation(async (command: string) => (command === 'account_update' ? { ...account, rights: ask } : []))
    document.querySelector<HTMLFormElement>('#account-form')!.requestSubmit()
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('account_update', expect.objectContaining({ id: 'a1', input: expect.objectContaining({ rights: ask }) }))
  })

  it('sets each kind of change on its own, and never lets sending or a far-reaching change go without approval', async () => {
    const wrapper = mount(AccountForm, { props: { account }, attachTo: document.body })
    mounted.push(wrapper)
    await flushPromises()
    const presets = levels('AI-clients mogen in dit account')
    // Vrij boeken: booking goes at once, the rest still asks.
    presets[2]!.click()
    await flushPromises()
    expect(['Boeken en wijzigen', 'Verwijderen', 'Versturen', 'Ingrijpend'].map(checked)).toEqual(['Zonder akkoord', 'Met akkoord', 'Met akkoord', 'Met akkoord'])
    expect(levels('Versturen').map((o) => o.textContent)).toEqual(['Uit', 'Met akkoord'])
    expect(levels('Ingrijpend').map((o) => o.textContent)).toEqual(['Uit', 'Met akkoord'])
    expect(levels('Verwijderen').map((o) => o.textContent)).toEqual(['Uit', 'Met akkoord', 'Zonder akkoord'])
    // Deleting off while booking stays free: no preset is that, and the form says it is an own choice.
    levels('Verwijderen')[0]!.click()
    await flushPromises()
    expect(presets.map((o) => o.getAttribute('aria-checked'))).toEqual(['false', 'false', 'false'])
    expect(document.querySelector('.rights')!.parentElement!.textContent).toContain('Eigen instelling')
    expect(document.querySelector('.note.bad')).toBeNull()
    // Deleting without approval comes with a warning.
    levels('Verwijderen')[2]!.click()
    await flushPromises()
    expect(document.querySelector('.note.bad')!.textContent).toContain('komt niet terug')
    levels('Verwijderen')[0]!.click()
    await flushPromises()
    invoke.mockImplementation(async (command: string) => (command === 'account_update' ? account : []))
    document.querySelector<HTMLFormElement>('#account-form')!.requestSubmit()
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('account_update', expect.objectContaining({ input: expect.objectContaining({ rights: { change: 'auto', delete: 'off', send: 'ask', heavy: 'ask' } }) }))
  })

  it('names the rights on the account in a few words', () => {
    expect(rightsChip({ change: 'off', delete: 'off', send: 'off', heavy: 'off' })).toBeNull()
    expect(rightsChip(ask)).toEqual({ label: 'Boekt met akkoord', tone: 'info' })
    expect(rightsChip({ ...ask, change: 'auto' })).toEqual({ label: 'Boekt zelf', tone: 'warn' })
    expect(rightsChip({ ...ask, change: 'auto', delete: 'auto' })).toEqual({ label: 'Boekt en verwijdert zelf', tone: 'warn' })
    expect(rightsChip({ ...ask, delete: 'auto' })).toEqual({ label: 'Verwijdert zelf', tone: 'warn' })
    expect(rightsChip({ ...ask, change: 'off' })).toEqual({ label: 'Beperkt met akkoord', tone: 'info' })
    expect(rightsText({ ...ask, change: 'auto', delete: 'off' })).toBe('Boeken en wijzigen: zonder akkoord · Verwijderen: uit · Versturen: met akkoord · Ingrijpend: met akkoord')
  })
})
