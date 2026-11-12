import { flushPromises, mount } from '@vue/test-utils'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { exportLog, log, type EbEntry, type Entry, type McpEntry } from '../lib/log'
import type { ApprovalDetail } from '../lib/approvals'
import { router } from '../router'
import RequestsView from '../views/RequestsView.vue'
import source from '../views/RequestsView.vue?raw'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))

beforeAll(() => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
})
afterAll(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

const mcp = (over: Partial<McpEntry> = {}): McpEntry => ({
  kind: 'mcp', id: 1, at: 1_000, client: 'Claude', tool: 'abonnementen_factureren', args: { maand: 11 }, account: 'Studio Noord',
  outcome: 'ok', durationMs: 640, bytes: 2048, error: null, calls: [2, 3], version: '0.4.12',
  waitStartedAt: null, waitEndedAt: null, waitedMs: 0, ...over,
})
const eb = (id: number, over: Partial<EbEntry> = {}): EbEntry => ({
  kind: 'eb', id, at: 1_000 + id, parent: 1, client: 'Claude', account: 'Studio Noord', method: 'POST', path: 'abonnement/factureren',
  query: [], status: 200, durationMs: 300, bytes: 512, rows: null, error: null, headers: [], truncated: false, version: '0.4.12', ...over,
})

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

let entries: Entry[] = []
/** What approval_get answers for a proposal, or why it cannot. */
let onApproval: (id: number) => Promise<Partial<ApprovalDetail>> = async (id) => ({ id, decision: 'approved', bulk: false })
let onExport: () => Promise<string> = async () => '/tmp/eb-mcp-export.json'
let onReveal: (path: string) => Promise<void> = async () => {}
let toolbar: HTMLElement
const mounted: { unmount(): void }[] = []

beforeEach(() => {
  entries = [mcp(), eb(2), eb(3, { version: '' })]
  invoke.mockReset()
  invoke.mockImplementation(async (command: string, args?: { id: number }) => {
    if (command === 'log_list') return entries
    if (command === 'log_get') return entries.find((e) => e.id === args?.id) ?? null
    if (command === 'log_export') return onExport()
    if (command === 'log_export_reveal') return onReveal((args as unknown as { path: string }).path)
    if (command === 'approval_get') return onApproval(args!.id)
    return undefined
  })
  // The page teleports its toolbar buttons into the shell's header; here the header is only this span.
  toolbar = document.createElement('span')
  toolbar.id = 'toolbar-actions'
  document.body.append(toolbar)
})
afterEach(() => {
  for (const wrapper of mounted.splice(0)) wrapper.unmount()
  toolbar.remove()
  log.entries = []
  log.loaded = false
  onExport = async () => '/tmp/eb-mcp-export.json'
  onReveal = async () => {}
  onApproval = async (id) => ({ id, decision: 'approved', bulk: false })
})

async function page() {
  const wrapper = mount(RequestsView, { global: { plugins: [router] }, attachTo: document.body })
  mounted.push(wrapper)
  await flushPromises()
  return wrapper
}
const exportButton = () => toolbar.querySelector<HTMLButtonElement>('button.export')!

describe('the request list', () => {
  it('gives every cell its own column class, the head included', async () => {
    const wrapper = await page()
    const columns = ['c-time', 'c-src', 'main', 'c-status', 'c-dur', 'c-size', 'c-who']
    const head = wrapper.find('.row.head')
    expect(head.findAll(':scope > span').map((cell) => cell.classes().find((c) => columns.includes(c)))).toEqual(columns)
    const row = wrapper.find('.row.item')
    expect(row.findAll(':scope > [role="cell"]').map((cell) => cell.classes().find((c) => columns.includes(c)))).toEqual(columns)
  })

  it('truncates a long tool name in its own cell, with the full name as tooltip', async () => {
    const wrapper = await page()
    const name = wrapper.find('.row.item .main .name')
    expect(name.text()).toBe('abonnementen_factureren')
    expect(name.attributes('title')).toBe('abonnementen_factureren')
    // The chip sits after the name in the same cell, with the full text as tooltip for when only the count shows.
    const chip = wrapper.find('.row.item .main .chip.calls')
    expect(chip.text()).toBe('2 aanroepen')
    expect(chip.attributes('title')).toBe('2 aanroepen')
    expect(chip.find('.unit').exists()).toBe(true)
    // The status label is its own truncating element, not loose text next to the dot.
    expect(wrapper.find('.row.item .c-status .clip').text()).toBe('OK')
  })

  it('puts the path of an e-Boekhouden call in a truncating span with a tooltip', async () => {
    const wrapper = await page()
    await wrapper.find('[role="tab"]:nth-child(3)').trigger('click')
    const path = wrapper.find('.row.item .main .path')
    expect(path.attributes('title')).toBe('abonnement/factureren')
  })

  it('sizes every column with minmax(0, ...) and gives up columns as the list narrows', () => {
    const style = source.slice(source.indexOf('<style'))
    const templates = [...style.matchAll(/\.row\s*\{[^}]*grid-template-columns:\s*([^;]+);/g)].map((m) => m[1]!.trim())
    // The full row, the full row with room for a wait in Duur, then without Grootte, without Wie, without Tijd.
    expect(templates.map((t) => t.match(/minmax\(/g)?.length)).toEqual([7, 7, 6, 5, 4])
    for (const template of templates) {
      const tracks = template.match(/minmax\([^)]*\)|\S+/g) ?? []
      for (const track of tracks) expect(track).toMatch(/^minmax\(0, [^)]+\)$/)
    }
    expect(style).toMatch(/\.row > \* \{ min-width: 0; \}/)
    expect(style).toMatch(/\.name, \.args, \.path \{[^}]*overflow: hidden;[^}]*text-overflow: ellipsis;[^}]*min-width: 0;/)
    expect(style).toMatch(/\.chip\.calls \{ flex: none;/)
    expect(style).toMatch(/\.list \{[^}]*container: requests \/ inline-size;/)
    expect(style).not.toMatch(/z-index/)
  })
})

describe('the detail of an entry', () => {
  it('names the app version that made it', async () => {
    const wrapper = await page()
    await wrapper.find('.row.item').trigger('click')
    await flushPromises()
    expect(wrapper.find('.facts .version').text()).toBe('App-versie 0.4.12')
  })

  it('leaves the version out for an entry from before versions were kept', async () => {
    entries = [mcp({ version: '' })]
    const wrapper = await page()
    await wrapper.find('.row.item').trigger('click')
    await flushPromises()
    expect(wrapper.find('.facts').exists()).toBe(true)
    expect(wrapper.find('.facts .version').exists()).toBe(false)
    expect(wrapper.find('.facts').text()).not.toContain('App-versie')
  })
})

describe('Exporteer', () => {
  it('calls log_export without arguments', async () => {
    await expect(exportLog()).resolves.toBe('/tmp/eb-mcp-export.json')
    expect(invoke).toHaveBeenCalledWith('log_export')
  })

  it('sits in the toolbar next to Leegmaken and shows it is busy while it runs', async () => {
    const run = deferred<string>()
    onExport = () => run.promise
    const wrapper = await page()
    expect([...toolbar.querySelectorAll('button')].map((b) => b.textContent?.trim())).toEqual(['Exporteer', 'Leegmaken'])
    expect(exportButton().querySelector('.btn-label')?.textContent).toBe('Exporteer')

    exportButton().click()
    await flushPromises()
    expect(exportButton().getAttribute('aria-busy')).toBe('true')
    expect(exportButton().disabled).toBe(true)
    expect(wrapper.find('.export-path').exists()).toBe(false)

    run.resolve('/Users/jan/Library/Logs/EB MCP/export-2026-11-11.json')
    await flushPromises()
    expect(exportButton().getAttribute('aria-busy')).toBe('false')
    expect(exportButton().disabled).toBe(false)
  })

  it('shows where it wrote the file, selectable, and reveals it in Finder', async () => {
    const path = '/Users/jan/Library/Logs/EB MCP/export-2026-11-11.json'
    onExport = async () => path
    const wrapper = await page()
    exportButton().click()
    await flushPromises()

    const note = wrapper.find('.export-note[role="status"]')
    expect(note.find('.export-path').text()).toBe(path)
    expect(note.find('.export-path').classes()).toContain('selectable')
    const reveal = note.find('button.reveal')
    // jsdom's user agent is no Windows one, so this is the macOS wording.
    expect(reveal.text()).toBe('Toon in Finder')

    const opening = deferred<void>()
    onReveal = () => opening.promise
    await reveal.trigger('click')
    expect(invoke).toHaveBeenCalledWith('log_export_reveal', { path })
    expect(reveal.attributes('aria-busy')).toBe('true')
    opening.resolve()
    await flushPromises()
    expect(reveal.attributes('aria-busy')).toBe('false')
    expect(wrapper.find('.reveal-error').exists()).toBe(false)

    await note.find('button.ghost').trigger('click')
    expect(wrapper.find('.export-note').exists()).toBe(false)
  })

  it('says so when the folder cannot be shown, and keeps the path', async () => {
    const wrapper = await page()
    exportButton().click()
    await flushPromises()
    onReveal = async () => {
      throw 'Dit exportbestand bestaat niet meer.'
    }
    await wrapper.find('button.reveal').trigger('click')
    await flushPromises()
    expect(wrapper.find('.reveal-error[role="alert"]').text()).toBe('Dit exportbestand bestaat niet meer.')
    expect(wrapper.find('.export-path').text()).toBe('/tmp/eb-mcp-export.json')
  })

  it('shows the reason when the export fails', async () => {
    onExport = async () => {
      throw 'De export kon niet worden opgeslagen: de schijf is vol.'
    }
    const wrapper = await page()
    exportButton().click()
    await flushPromises()
    const alert = wrapper.find('.note.bad[role="alert"]')
    expect(alert.text()).toContain('De export kon niet worden opgeslagen: de schijf is vol.')
    expect(wrapper.find('.export-path').exists()).toBe(false)
    expect(exportButton().getAttribute('aria-busy')).toBe('false')

    // A later export that works replaces the failure.
    onExport = async () => '/tmp/eb-mcp-export.json'
    exportButton().click()
    await flushPromises()
    expect(wrapper.find('.note.bad[role="alert"]').exists()).toBe(false)
    expect(wrapper.find('.export-path').text()).toBe('/tmp/eb-mcp-export.json')
  })
})

/** A change that waited 47,3 s for approval: 400 ms preparing before it, 600 ms making it after. */
const waitedCall = (over: Partial<McpEntry> = {}): McpEntry =>
  mcp({ id: 10, at: 100_000, tool: 'mutatie_toevoegen', proposal: 7, calls: [11, 12], waitStartedAt: 100_400, waitEndedAt: 147_700, waitedMs: 47_300, durationMs: 48_300, ...over })
const prepCall = eb(11, { parent: 10, at: 100_100, method: 'GET', path: 'grootboekrekening/gridtable', durationMs: 250 })
const execCall = eb(12, { parent: 10, at: 147_750, method: 'POST', path: 'mutatie', durationMs: 500 })

describe('the duration of a call that waited for approval', () => {
  const durOf = (wrapper: Awaited<ReturnType<typeof page>>, index = 0) => wrapper.findAll('.row.item:not(.child)')[index]!.find('.c-dur')

  it('shows the work time, with the wait muted beside it, for an approved call', async () => {
    entries = [waitedCall(), prepCall, execCall]
    const wrapper = await page()
    const cell = durOf(wrapper)
    expect(cell.find('.clip').text()).toBe('1,00 s')
    expect(cell.find('.waited').text()).toBe('+ 47,3 s gewacht')
    expect(cell.attributes('title')).toBe('1,00 s aan het werk, 47,3 s gewacht op akkoord')
    // Blue, striped amber and green, on the work scale. Its 1 s of work is the longest listed, so the work fills the
    // track but for the 4% the wait keeps, in the 40 to 60 split of preparing and making the change.
    const parts = cell.findAll('.bar.phased .seg')
    expect(parts.map((p) => p.classes().find((c) => ['prep', 'wait', 'exec'].includes(c)))).toEqual(['prep', 'wait', 'exec'])
    expect(parts.map((p) => Math.round(parseFloat(p.attributes('style')!.match(/width: ([\d.]+)%/)![1]!)))).toEqual([38, 4, 58])
  })

  it('shows the preparing only, with the wait, for a refused call', async () => {
    entries = [waitedCall({ outcome: 'refused', waitEndedAt: 112_400, waitedMs: 12_000, durationMs: 12_400, calls: [11], error: 'De gebruiker heeft dit geweigerd in EB MCP. Er is niets gewijzigd.' }), prepCall]
    const wrapper = await page()
    const cell = durOf(wrapper)
    expect(cell.find('.clip').text()).toBe('400 ms')
    expect(cell.find('.waited').text()).toBe('+ 12,0 s gewacht')
    expect(cell.findAll('.bar.phased .seg').map((p) => p.classes().find((c) => ['prep', 'wait', 'exec'].includes(c)))).toEqual(['prep', 'wait'])
  })

  it('keeps counting a wait that still runs', async () => {
    const started = Date.now() - 4 * 60_000 - 5_000
    entries = [waitedCall({ outcome: 'waiting', at: started - 400, waitStartedAt: started, waitEndedAt: null, waitedMs: 0, durationMs: 0, calls: [] })]
    const wrapper = await page()
    const cell = durOf(wrapper)
    expect(cell.find('.clip').text()).toBe('400 ms')
    expect(cell.find('.waited').text()).toBe('wacht al 4 min')
  })

  it('shows the legend only while a listed call waited, and scales the bars on work time', async () => {
    entries = [mcp({ id: 1, durationMs: 2_000 }), waitedCall(), prepCall, execCall]
    const wrapper = await page()
    expect(wrapper.find('.legend').exists()).toBe(true)
    expect(wrapper.findAll('.legend li').map((li) => li.text())).toEqual(['Voorbereiden', 'Wacht op akkoord', 'Uitvoeren in e-Boekhouden'])
    expect(wrapper.find('.list').classes()).toContain('waits')
    // The plain call of 2 s fills its track; the 48 s span of the waited one does not shrink it.
    const plain = durOf(wrapper, 0).find('.bar:not(.phased) span')
    expect(plain.attributes('style')).toContain('width: 100%')

    // Filtered down to the plain call: no wait listed, no legend.
    const search = wrapper.find('input[type="search"]')
    await search.setValue('abonnementen')
    await flushPromises()
    expect(wrapper.findAll('.row.item:not(.child)')).toHaveLength(1)
    expect(wrapper.find('.legend').exists()).toBe(false)
    expect(wrapper.find('.list').classes()).not.toContain('waits')
  })

  it('leaves a call without a wait as it was', async () => {
    const wrapper = await page()
    const cell = durOf(wrapper)
    expect(cell.find('.clip').text()).toBe('640 ms')
    expect(cell.find('.waited').exists()).toBe(false)
    expect(cell.find('.bar.phased').exists()).toBe(false)
    expect(cell.find('.bar span').attributes('style')).toContain('width: 64%')
    expect(wrapper.find('.legend').exists()).toBe(false)
    await wrapper.find('.row.item').trigger('click')
    await flushPromises()
    expect(wrapper.find('.wait-view').exists()).toBe(false)
    expect(wrapper.find('.facts .chip:nth-child(2)').text()).toBe('640 ms')
    expect(invoke).not.toHaveBeenCalledWith('approval_get', expect.anything())
  })

  it('lets the wait text give way first and only shows it on a wide list', () => {
    const style = source.slice(source.indexOf('<style'))
    expect(style).toMatch(/\.waited \{ display: none; flex: 0 1000 auto; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;/)
    expect(style).toMatch(/@container requests \(min-width: 900px\) \{\s*\.list\.waits \.row \{[^}]*\}\s*\.waited \{ display: block; \}/)
    expect(style).toMatch(/\.c-who, \.bar, \.bar\.phased \{ display: none; \}/)
  })
})

describe('the detail of a call that waited for approval', () => {
  async function opened() {
    entries = [waitedCall(), prepCall, execCall]
    const wrapper = await page()
    await wrapper.find('.row.item').trigger('click')
    await flushPromises()
    return wrapper
  }

  it('shows the figures, the large bar and the steps with the calls of each phase', async () => {
    const wrapper = await opened()
    expect(invoke).toHaveBeenCalledWith('approval_get', { id: 7 })
    const view = wrapper.find('.wait-view')
    expect(view.findAll('.figure').map((f) => [f.find('dt').text(), f.find('dd').text()])).toEqual([
      ['Aan het werk', '1,00 s'], ['Gewacht', '47,3 s'], ['Totaal', '48,30 s'],
    ])
    expect(view.findAll('.phase-bar .seg').map((p) => p.classes().find((c) => ['prep', 'wait', 'exec'].includes(c)))).toEqual(['prep', 'wait', 'exec'])
    const steps = view.findAll('.steps > li')
    expect(steps.map((s) => s.attributes('data-step'))).toEqual(['received', 'prep', 'answered', 'wait', 'approved', 'exec', 'done'])
    expect(steps.map((s) => s.find('.step-title').text())).toEqual(['Aanroep ontvangen', 'Voorbereiden', 'Antwoord aan Claude', 'Wacht op akkoord', 'Goedgekeurd', 'Uitvoeren', 'Klaar'])
    expect(steps[1]!.findAll('.step-call .path').map((p) => p.text())).toEqual(['grootboekrekening/gridtable'])
    expect(steps[1]!.find('.step-call').text()).toContain('GET')
    expect(steps[1]!.find('.step-call').text()).toContain('200')
    expect(steps[1]!.find('.step-call').text()).toContain('250 ms')
    expect(steps[5]!.findAll('.step-call .path').map((p) => p.text())).toEqual(['mutatie'])
    expect(steps[2]!.find('.step-note').text()).toBe('Status wacht, voorstel 7')
    expect(steps[3]!.find('.step-chip').text()).toBe('niet meegeteld')
    expect(steps[4]!.find('.step-value').text()).not.toContain('Alles goedkeuren')
    // The existing parts stay, the waterfall on work time.
    expect(wrapper.findAll('.detail-body .section-label').map((h) => h.text())).toContain('Argumenten')
    expect(wrapper.findAll('.wf')).toHaveLength(2)
    expect(wrapper.find('.facts .chip:nth-child(2)').text()).toBe('1,00 s')
  })

  it('names an approval given with Alles goedkeuren', async () => {
    onApproval = async (id) => ({ id, decision: 'approved', bulk: true })
    const wrapper = await opened()
    expect(wrapper.find('[data-step="approved"] .step-value').text()).toMatch(/ · Alles goedkeuren$/)
  })

  it('opens the record on Akkoord with Bekijk voorstel op Akkoord', async () => {
    await router.push('/verzoeken')
    const wrapper = await opened()
    const button = wrapper.find('button.open-proposal')
    expect(button.text()).toBe('Bekijk voorstel op Akkoord')
    await button.trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.fullPath).toBe('/akkoord?open=7')
  })

  it('still shows the steps when the record left the history', async () => {
    onApproval = async () => {
      throw 'Dit voorstel bestaat niet meer.'
    }
    const wrapper = await opened()
    expect(wrapper.find('[data-step="approved"] .step-title').text()).toBe('Goedgekeurd')
  })
})
