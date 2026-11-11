import { flushPromises, mount } from '@vue/test-utils'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { exportLog, log, type EbEntry, type Entry, type McpEntry } from '../lib/log'
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
  outcome: 'ok', durationMs: 640, bytes: 2048, error: null, calls: [2, 3], version: '0.4.12', ...over,
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
})

async function page() {
  const wrapper = mount(RequestsView, { attachTo: document.body })
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
    // The full row, then without Grootte, without Wie, without Tijd.
    expect(templates.map((t) => t.match(/minmax\(/g)?.length)).toEqual([7, 6, 5, 4])
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
