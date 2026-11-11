import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'
import { filterOpItems, opLabel, type OpItem } from '../lib/accounts'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))

const ITEMS: OpItem[] = [
  { reference: 'op://v1/i1', title: 'e-Boekhouden Studio Noord', vault: 'Werk', username: 'jan@studionoord.example', suggested: true },
  { reference: 'op://v2/i2', title: 'e-Boekhouden (Praktijk West)', vault: 'Privé', username: 'praktijk@west.example', suggested: true },
  { reference: 'op://v3/i3', title: 'Boekhoudpakket oud', vault: 'Archief', username: 'jan@studionoord.example', suggested: false },
  { reference: 'op://v1/i4', title: 'Zakelijke bank', vault: 'Werk', username: 'jan', suggested: false },
]

describe('1Password items', () => {
  it('puts e-Boekhouden items in their own group and searches title, username and vault', () => {
    const all = filterOpItems(ITEMS, '')
    expect(all.suggested.map((i) => i.reference)).toEqual(['op://v1/i1', 'op://v2/i2'])
    expect(all.other.map((i) => i.reference)).toEqual(['op://v3/i3', 'op://v1/i4'])

    expect(filterOpItems(ITEMS, 'studionoord').other.map((i) => i.title)).toEqual(['Boekhoudpakket oud'])
    const werk = filterOpItems(ITEMS, '  WERK  bank ')
    expect([...werk.suggested, ...werk.other].map((i) => i.title)).toEqual(['Zakelijke bank'])
    expect(filterOpItems(ITEMS, 'niets hiervan')).toEqual({ suggested: [], other: [] })
  })

  it('names a picked item by vault and title, else by its reference', () => {
    expect(opLabel({ opRef: 'op://v1/i1', opTitle: 'Studio Noord', opVault: 'Werk' })).toBe('Werk · Studio Noord')
    expect(opLabel({ opRef: 'op://v1/i1', opTitle: 'Studio Noord', opVault: null })).toBe('Studio Noord')
    expect(opLabel({ opRef: 'op://Werk/EB', opTitle: null, opVault: null })).toBe('op://Werk/EB')
  })
})

describe('item picker', () => {
  afterEach(() => invoke.mockReset())

  async function mountPicker() {
    const { default: OpItemPicker } = await import('../components/OpItemPicker.vue')
    const wrapper = mount(OpItemPicker, { attachTo: document.body })
    await flushPromises()
    return wrapper
  }

  it('lists the items in two groups and picks with the arrow keys and Enter', async () => {
    invoke.mockResolvedValue(ITEMS)
    const wrapper = await mountPicker()
    expect(invoke).toHaveBeenCalledWith('op_items')
    expect(wrapper.findAll('.group-label').map((l) => l.text())).toEqual(['Voor e-Boekhouden', 'Andere logins'])
    expect(wrapper.text()).toContain('4 van 4 logins')
    expect(wrapper.text()).not.toMatch(/wachtwoord/i)

    const search = wrapper.find('input')
    await search.setValue('boek')
    expect(wrapper.text()).toContain('3 van 4 logins')
    await search.trigger('keydown', { key: 'ArrowDown' })
    await search.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('pick')?.[0]?.[0]).toMatchObject({ reference: 'op://v2/i2' })
    wrapper.unmount()
  })

  it('closes on Esc without letting the sheet close too', async () => {
    invoke.mockResolvedValue(ITEMS)
    const wrapper = await mountPicker()
    const sheetClose = vi.fn()
    document.addEventListener('keydown', sheetClose)
    await wrapper.find('input').trigger('keydown', { key: 'Escape' })
    expect(wrapper.emitted('close')).toHaveLength(1)
    expect(sheetClose).not.toHaveBeenCalled()
    document.removeEventListener('keydown', sheetClose)
    wrapper.unmount()
  })

  it('closes only the list on Esc when it sits in a sheet, and the next Esc closes the sheet', async () => {
    invoke.mockResolvedValue(ITEMS)
    const { default: OpItemPicker } = await import('../components/OpItemPicker.vue')
    const { default: Sheet } = await import('../components/Sheet.vue')
    const pickerClose = vi.fn()
    const wrapper = mount(Sheet, {
      props: { title: 'Account toevoegen' },
      slots: { default: () => h(OpItemPicker, { onClose: pickerClose }) },
      attachTo: document.body,
    })
    await flushPromises()
    const search = wrapper.find('input').element as HTMLInputElement
    search.focus()
    search.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(pickerClose).toHaveBeenCalledTimes(1)
    expect(wrapper.emitted('close')).toBeUndefined()

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
  })

  it('explains a missing CLI and a locked 1Password, and tries again', async () => {
    invoke.mockRejectedValueOnce({ kind: 'missing' })
    const wrapper = await mountPicker()
    expect(wrapper.text()).toContain('niet gevonden')
    expect(wrapper.text()).toContain('Installatie-uitleg')

    invoke.mockRejectedValueOnce({ kind: 'locked' })
    await wrapper.findAll('button').find((b) => b.text().includes('Opnieuw zoeken'))!.trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('vergrendeld')
    expect(wrapper.text()).toContain('Opnieuw proberen')

    invoke.mockResolvedValueOnce(ITEMS)
    await wrapper.findAll('button').find((b) => b.text().includes('Opnieuw proberen'))!.trigger('click')
    await flushPromises()
    expect(wrapper.findAll('[role="option"]')).toHaveLength(4)
    wrapper.unmount()
  })
})
