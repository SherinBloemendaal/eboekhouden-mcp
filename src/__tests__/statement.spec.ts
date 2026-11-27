import { flushPromises, mount } from '@vue/test-utils'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '../App.vue'
import { store } from '../lib/accounts'
import {
  approvalState, decisionLabel, dutchDate, euroCents, filterRows, outReason, proposalKind, statusOf, testHits,
  type ApprovalRecord, type Proposal, type StatementEdit, type StatementRow, type StatementRule, type StatementView,
} from '../lib/approvals'
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

function row(line: number, description: string, cents: number, over: Partial<StatementRow> = {}): StatementRow {
  return { line, cells: ['1111', `2026-10-${10 + line}`, description, String(cents)], date: `2026-10-${10 + line}`, description, cents, included: true, out: null, overruled: null, ...over }
}

const RULE: StatementRule = { kind: 'contains', text: 'BETALING ONTVANGEN', on: true, by: 'client', why: 'al geboekt', label: 'Omschrijving bevat BETALING ONTVANGEN', count: 1 }

function statement(over: Partial<StatementView> = {}): StatementView {
  return {
    fileName: 'kaart.csv', header: ['Kaartnummer', 'Transactiedatum', 'Omschrijving', 'Bedrag in EUR'], delimiter: 'komma', empty: 2,
    mapping: { date: 1, description: 2, amount: 3, start: 2 }, rules: [RULE], dedupe: true,
    ledgers: [{ id: 11, code: '1010', name: 'Bank' }, { id: 18, code: '1820', name: 'Credit card' }], ledger: 1,
    rows: [
      row(1, 'Omschrijving', 0, { date: null, cents: null, included: false, out: { kind: 'beforeStart' } }),
      row(2, 'CURSOR USAGE', -4826),
      row(3, 'BETALING ONTVANGEN - DANK U', 200004, { included: false, out: { kind: 'rule', rule: 0 } }),
      row(4, 'Thuisbezorgd.nl Utrecht', -3145),
      row(5, 'totaal', 0, { date: null, cents: null, included: false, out: { kind: 'unreadable', what: 'geen datum' } }),
    ],
    totals: { lines: 3, included: 2, skipped: 1, unreadable: 1, cents: -7971, first: '2026-10-12', last: '2026-10-14', perRule: [1] },
    ...over,
  }
}

function proposal(over: Partial<Proposal> = {}): Proposal {
  return {
    id: 21, accountId: 'a1', account: 'Studio Noord', client: 'Claude', tool: 'afschrift_inlezen', title: 'Afschrift inlezen: kaart.csv',
    lines: [{ label: 'Gaat mee', value: '2 regels' }], danger: true, askedAt: Date.now() - 60_000,
    statement: { fileName: 'kaart.csv', lines: 3, included: 2 }, ...over,
  }
}

let waiting: Proposal[] = []
let history: ApprovalRecord[] = []
let current = statement()
let onEdit: (edit: StatementEdit) => StatementView | Promise<StatementView> = () => current

const mounted: { unmount(): void }[] = []
beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case 'approvals_list': return waiting
      case 'approvals_history': return history
      case 'approval_statement': return current
      case 'approval_statement_edit': return onEdit(args!.edit as StatementEdit)
      case 'approval_decide':
      case 'approval_return':
        waiting = waiting.filter((p) => p.id !== args!.id)
        return undefined
      case 'accounts_list': return []
      case 'settings_get': return { idleMinutes: 30, appearance: 'system', showInDock: true, notifyApprovals: true, waitForApproval: false, waitMinutes: 5 }
      case 'log_list': return []
      default: return undefined
    }
  })
})
afterEach(() => {
  for (const wrapper of mounted.splice(0)) wrapper.unmount()
  store.approvals = []
  approvalState.history = []
  waiting = []
  history = []
  current = statement()
  onEdit = () => current
})

async function mountAt(path: string) {
  await router.push(path)
  await router.isReady()
  const wrapper = mount(App, { global: { plugins: [router] }, attachTo: document.body })
  mounted.push(wrapper)
  await flushPromises()
  return wrapper
}

const button = (root: ParentNode, text: string) => [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)
const detail = () => document.querySelector<HTMLElement>('#q-detail')!
const sheet = () => document.querySelector<HTMLElement>('[aria-label="Afschrift controleren"]')
const calls = (command: string) => invoke.mock.calls.filter((c) => c[0] === command).map((c) => c[1])
const lines = () => [...sheet()!.querySelectorAll<HTMLElement>('.rows .tr')]

async function openSheet() {
  waiting = [proposal()]
  await mountAt('/akkoord')
  button(detail(), 'Regels controleren')!.click()
  await flushPromises()
}

describe('a statement on Akkoord', () => {
  it('says that the user chooses what goes along, and names the button by what it does', async () => {
    waiting = [proposal()]
    await mountAt('/akkoord')
    expect(detail().querySelector('.statement-note')?.textContent).toContain('Wat meegaat kies jij hier, niet de AI-client')
    expect(button(detail(), 'Inlezen (2)')).toBeTruthy()
    expect(button(detail(), 'Goedkeuren')).toBeUndefined()
    // Reading in is not shown as something that cannot be put right: the lines can be removed again.
    expect(button(detail(), 'Inlezen (2)')!.classList.contains('primary')).toBe(true)
    expect(sheet()).toBeNull()
    expect(proposalKind(proposal())).toEqual({ icon: 'list-checks', tone: 'danger' })
  })

  it('shows every line from the start line on, with why a line stays out', async () => {
    await openSheet()
    expect(calls('approval_statement')).toEqual([{ id: 21 }])
    // The header line is before the start line and is not listed.
    expect(lines().map((l) => l.querySelector('.line')?.textContent)).toEqual(['2', '3', '4', '5'])
    expect(lines().map((l) => l.querySelector('.chip')?.textContent ?? null)).toEqual([null, 'Omschrijving bevat BETALING ONTVANGEN', null, 'Niet te lezen: geen datum'])
    expect(lines()[1]!.classList.contains('off')).toBe(true)
    expect(lines()[0]!.querySelector('.amount')?.textContent).toBe('-48,26')
    // A line without a date cannot be ticked.
    expect(lines()[3]!.querySelector<HTMLButtonElement>('.box')!.disabled).toBe(true)
    expect(sheet()!.querySelector('.summary')?.textContent).toContain('2 regels gaan mee')
    expect(sheet()!.querySelector('.summary')?.textContent).toContain('12-10-2026 t/m 14-10-2026 · samen -79,71 · 1 overgeslagen · 1 niet te lezen')
    expect(sheet()!.querySelector('.rule')?.textContent).toContain('1 regel · al geboekt')
    expect(sheet()!.querySelector('.rule .chip')?.textContent).toBe('Claude')
    expect(sheet()!.querySelector('.foot-note')?.textContent).toContain('2 lege regels in het bestand tellen niet mee')
    // The tabs narrow the list.
    button(sheet()!, 'Overgeslagen2')!.click()
    await flushPromises()
    expect(lines().map((l) => l.querySelector('.line')?.textContent)).toEqual(['3', '5'])
  })

  it('sends each change to the app and shows what comes back', async () => {
    await openSheet()
    const edits: StatementEdit[] = []
    onEdit = (edit) => {
      edits.push(edit)
      return statement({ totals: { ...current.totals, included: 1, skipped: 2 } })
    }
    lines()[0]!.querySelector<HTMLButtonElement>('.box')!.click()
    await flushPromises()
    sheet()!.querySelector<HTMLButtonElement>('.rule .switch')!.click()
    await flushPromises()
    const amount = sheet()!.querySelector<HTMLSelectElement>('#st-amount')!
    amount.value = '0'
    amount.dispatchEvent(new Event('change'))
    await flushPromises()
    expect(edits).toEqual([
      { kind: 'tick', lines: [2], included: false },
      { kind: 'toggleRule', index: 0, on: false },
      { kind: 'mapping', mapping: { date: 1, description: 2, amount: 0, start: 2 } },
    ])
    expect(button(sheet()!, 'Inlezen (1)')).toBeTruthy()
    // Nothing is approved by editing.
    expect(calls('approval_decide')).toEqual([])
  })

  it('lights up the lines a new rule would leave out before it is added', async () => {
    await openSheet()
    button(sheet()!, 'Regel toevoegen')!.click()
    await flushPromises()
    const text = sheet()!.querySelector<HTMLInputElement>('#st-text')!
    expect(sheet()!.querySelector('.adder-hits')?.textContent).toBe('Vul in waar de regel op let.')
    text.value = 'thuisbezorgd'
    text.dispatchEvent(new Event('input'))
    await flushPromises()
    expect(sheet()!.querySelector('.adder-hits')?.textContent).toContain('Raakt 1 regel, samen -31,45')
    expect(lines().filter((l) => l.classList.contains('hit')).map((l) => l.querySelector('.line')?.textContent)).toEqual(['4'])
    const edits: StatementEdit[] = []
    onEdit = (edit) => {
      edits.push(edit)
      return current
    }
    sheet()!.querySelector<HTMLFormElement>('.adder')!.dispatchEvent(new Event('submit', { cancelable: true }))
    await flushPromises()
    expect(edits).toEqual([{ kind: 'addRule', test: { kind: 'contains', text: 'thuisbezorgd' } }])
    expect(sheet()!.querySelector('.adder')).toBeNull()
  })

  it('says why an edit was refused and keeps the sheet as it was', async () => {
    await openSheet()
    onEdit = () => Promise.reject('Datum, omschrijving en bedrag moeten elk een eigen kolom hebben')
    const date = sheet()!.querySelector<HTMLSelectElement>('#st-date')!
    date.value = '3'
    date.dispatchEvent(new Event('change'))
    await flushPromises()
    expect(sheet()!.querySelector('[role="alert"]')?.textContent).toBe('Datum, omschrijving en bedrag moeten elk een eigen kolom hebben')
    expect(button(sheet()!, 'Inlezen (2)')).toBeTruthy()
  })

  it('reads in only on Inlezen, and the sheet leaves with its proposal', async () => {
    await openSheet()
    button(sheet()!, 'Inlezen (2)')!.click()
    await flushPromises()
    expect(calls('approval_decide')).toEqual([{ id: 21, approve: true }])
    expect(sheet()).toBeNull()
  })
})

describe('sending a proposal back to the AI client', () => {
  it('sends the remark from the sheet, and nothing is approved or refused', async () => {
    await openSheet()
    button(sheet()!, 'Terug naar Claude')!.click()
    await flushPromises()
    const send = button(sheet()!, 'Stuur naar Claude')!
    // Without a remark the AI client would not know what to change.
    expect(send.disabled).toBe(true)
    expect(sheet()!.querySelector('.hint')?.textContent).toContain('samen met wat je in dit scherm hebt aangepast')
    const field = sheet()!.querySelector<HTMLTextAreaElement>('#return-remark')!
    field.value = 'De Thuisbezorgd-regels zijn privé.'
    field.dispatchEvent(new Event('input'))
    await flushPromises()
    send.click()
    await flushPromises()
    expect(calls('approval_return')).toEqual([{ id: 21, remark: 'De Thuisbezorgd-regels zijn privé.' }])
    expect(calls('approval_decide')).toEqual([])
    expect(sheet()).toBeNull()
    expect(document.querySelector('.feedback-text')?.textContent).toContain('Voorstel 21 is teruggestuurd naar Claude met je opmerking')
  })

  it('works for any proposal from the detail, and closes without sending on Annuleren', async () => {
    waiting = [proposal({ id: 7, tool: 'mutatie_toevoegen', title: 'Mutatie toevoegen: Geld uitgegeven', danger: false, statement: undefined })]
    await mountAt('/akkoord')
    expect(detail().querySelector('.statement-note')).toBeNull()
    button(detail(), 'Terug naar Claude')!.click()
    await flushPromises()
    expect(detail().querySelector('.hint')?.textContent).not.toContain('in dit scherm')
    button(detail(), 'Annuleren')!.click()
    await flushPromises()
    expect(detail().querySelector('#return-remark')).toBeNull()
    expect(calls('approval_return')).toEqual([])
    expect(button(detail(), 'Goedkeuren')).toBeTruthy()
  })

  it('shows a proposal that went back in the history, with the remark', async () => {
    history = [{
      ...proposal(), decidedAt: Date.now(), decision: 'returned', status: 'returned', version: '0.6.9', bulk: false,
      error: 'De gebruiker heeft dit voorstel teruggestuurd', remark: 'De Thuisbezorgd-regels zijn privé.',
    }]
    await mountAt('/akkoord')
    button(document, 'Geschiedenis')!.click()
    await flushPromises()
    const record = document.querySelector<HTMLButtonElement>('#record-21')!
    expect(record.querySelector('.chip.status')?.textContent).toBe('Teruggestuurd')
    expect(record.querySelector('.chip.status')?.classList.contains('back')).toBe(true)
    record.click()
    await flushPromises()
    expect(document.querySelector('.note.outcome')?.textContent).toContain('Teruggestuurd naar Claude. Er is niets gewijzigd in e-Boekhouden.')
    expect(document.querySelector('.remark')?.textContent).toBe('De Thuisbezorgd-regels zijn privé.')
    expect(statusOf('returned')).toEqual({ value: 'returned', label: 'Teruggestuurd', tone: 'back' })
    expect(decisionLabel('returned')).toBe('Teruggestuurd naar de AI-client')
  })
})

describe('the statement helpers', () => {
  const rows = statement().rows

  it('writes cents and dates the Dutch way', () => {
    expect([euroCents(-941237), euroCents(5), euroCents(100000000), euroCents(0)]).toEqual(['-9.412,37', '0,05', '1.000.000,00', '0,00'])
    expect([dutchDate('2026-11-06'), dutchDate(null)]).toEqual(['06-11-2026', ''])
  })

  it('filters the lines by tab and by every word typed', () => {
    expect(filterRows(rows, 'all', '').map((r) => r.line)).toEqual([2, 3, 4, 5])
    expect(filterRows(rows, 'in', '').map((r) => r.line)).toEqual([2, 4])
    expect(filterRows(rows, 'out', '').map((r) => r.line)).toEqual([3, 5])
    // Any cell counts, also one that is not a chosen column.
    expect(filterRows(rows, 'all', 'utrecht THUIS').map((r) => r.line)).toEqual([4])
    expect(filterRows(rows, 'all', '1111 dank').map((r) => r.line)).toEqual([3])
  })

  it('says why a line stays out', () => {
    expect(rows.map((r) => outReason(r.out, [RULE]))).toEqual(['Voor de startregel', '', 'Omschrijving bevat BETALING ONTVANGEN', '', 'Niet te lezen: geen datum'])
    expect(outReason({ kind: 'user' }, [])).toBe('Zelf uitgezet')
  })

  it('tells which lines a rule in the making would leave out', () => {
    const hit = (test: Parameters<typeof testHits>[0]) => rows.filter((r) => testHits(test, r)).map((r) => r.line)
    expect(hit({ kind: 'contains', text: ' betaling ' })).toEqual([3])
    expect(hit({ kind: 'contains', text: ' ' })).toEqual([])
    expect(hit({ kind: 'until', date: [2026, 10, 12] })).toEqual([2])
    expect(hit({ kind: 'from', date: [2026, 10, 13] })).toEqual([3, 4])
    expect(hit({ kind: 'column', column: 0, value: '1111' })).toEqual([2, 3, 4])
    expect(hit({ kind: 'amount', cents: -3145 })).toEqual([4])
    expect(hit({ kind: 'incoming' })).toEqual([3])
    expect(hit({ kind: 'outgoing' })).toEqual([2, 4])
    expect(hit({ kind: 'cancelling' })).toEqual([])
  })
})
