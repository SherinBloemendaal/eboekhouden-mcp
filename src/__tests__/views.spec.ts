import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'
import { router } from '../router'
import App from '../App.vue'
import Alert from '../components/Alert.vue'
import ClientIcon from '../components/ClientIcon.vue'
import Sheet from '../components/Sheet.vue'
import { store, type AccountView } from '../lib/accounts'
import { CLIENT_NAMES, SERVER_NAMES, usesBridge, type Client } from '../lib/connect'
import { unlockName } from '../lib/server'

// Every page is mounted in the document, as in the app: the toolbar's Teleport target must be findable.
// All of them share one router, so a page left mounted would follow the next test's route: each is taken down.
const mounted: { unmount(): void }[] = []
async function mountAt(path: string) {
  router.push(path)
  await router.isReady()
  const wrapper = mount(App, { global: { plugins: [router] }, attachTo: document.body })
  mounted.push(wrapper)
  await flushPromises()
  return wrapper
}
afterEach(() => {
  for (const wrapper of mounted.splice(0)) wrapper.unmount()
})

describe('views', () => {
  it('shows the four pages in the sidebar, Dashboard first, and Instellingen at the foot', async () => {
    const wrapper = await mountAt('/dashboard')
    const labels = wrapper.findAll('nav a').map((a) => a.text())
    expect(labels).toEqual(['Dashboard', 'Accounts', 'Verzoeken', 'Koppelen', 'Instellingen⌘,'])
    expect(wrapper.find('header h1').text()).toBe('Dashboard')
  })

  it('opens Instellingen with Cmd+, and offers Systeem, Licht and Donker', async () => {
    const wrapper = await mountAt('/dashboard')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: ',', metaKey: true }))
    await flushPromises()
    expect(router.currentRoute.value.path).toBe('/instellingen')
    expect(wrapper.find('header h1').text()).toBe('Instellingen')
    expect(wrapper.findAll('[role="radio"]').map((b) => b.text())).toEqual(['Systeem', 'Licht', 'Donker'])
    expect(wrapper.find('[role="radio"][aria-checked="true"]').text()).toBe('Systeem')
    wrapper.unmount()
  })

  it('opens the menu-bar panel with its items and Afsluiten', async () => {
    const wrapper = await mountAt('/panel')
    const items = wrapper.findAll('button.item').map((b) => b.text())
    expect(items[0]).toBe('Dashboard')
    expect(items.at(-1)).toContain('Afsluiten')
    expect(wrapper.text()).toContain('Niet ingelogd')
  })

  it('shows the server address, a masked token and the tool count from the server on Koppelen', async () => {
    const wrapper = await mountAt('/koppelen')
    expect(wrapper.text()).toContain('http://127.0.0.1:47821/mcp')
    expect(wrapper.text()).toContain('••••')
    expect(wrapper.find('.chip.mcp').text()).toMatch(/^\d+ tools?$/)
    wrapper.unmount()
  })
})

function account(over: Partial<AccountView>): AccountView {
  return {
    id: 'a1', label: 'Studio Noord', username: 'jan@studionoord.example', store: 'system', opRef: null, opTitle: null, opVault: null,
    administration: 'Studio Noord B.V.', session: null, signingIn: false, ...over,
  }
}

describe('accounts page', () => {
  afterEach(() => {
    store.accounts = []
    store.signins = {}
    vi.restoreAllMocks()
  })

  it('shows the empty state with a way to add an account', async () => {
    const wrapper = await mountAt('/accounts')
    expect(wrapper.text()).toContain('Nog geen accounts')
    expect(wrapper.find('button.primary').text()).toContain('Account toevoegen')
    wrapper.unmount()
  })

  it('names the unlock of this system and how long a session runs', async () => {
    const wrapper = await mountAt('/accounts')
    store.accounts = [account({ session: { accountId: 'a1', label: 'Studio Noord', administration: null, startedAt: Date.now(), expired: false } })]
    await flushPromises()
    expect(wrapper.text()).toContain(`${unlockName} bij elk gebruik`)
    expect(wrapper.find('.session').text()).toMatch(/ingelogd/i)
    wrapper.unmount()
  })

  it('offers to follow a sign-in started elsewhere and shows its steps', async () => {
    const wrapper = await mountAt('/accounts')
    store.accounts = [account({ signingIn: true })]
    store.signins = { a1: { accountId: 'a1', step: 'code', message: null } }
    await flushPromises()
    const follow = wrapper.find('.actions .btn')
    expect(follow.text()).toBe('Volgen')
    expect(follow.attributes('disabled')).toBeUndefined()
    await follow.trigger('click')
    await flushPromises()
    const sheet = wrapper.find('[role="dialog"]')
    expect(sheet.attributes('aria-label')).toBe('Inloggen bij Studio Noord')
    expect(sheet.text()).toContain('Tweestapsverificatie')
    wrapper.unmount()
  })

  it('stops its clock when the page closes', async () => {
    const setSpy = vi.spyOn(globalThis, 'setInterval')
    const clearSpy = vi.spyOn(globalThis, 'clearInterval')
    const wrapper = await mountAt('/accounts')
    const index = setSpy.mock.calls.findIndex((call) => call[1] === 30_000)
    expect(index).toBeGreaterThanOrEqual(0)
    const id = setSpy.mock.results[index]?.value
    wrapper.unmount()
    expect(clearSpy).toHaveBeenCalledWith(id)
  })
})

describe('koppelen', () => {
  it('draws a logo for every client, Codex included', () => {
    const clients = Object.keys(CLIENT_NAMES) as Client[]
    expect(clients).toEqual(['claude', 'claudeCode', 'cursor', 'codex'])
    for (const client of clients) {
      const wrapper = mount(ClientIcon, { props: { client } })
      expect(wrapper.find('svg path').exists()).toBe(true)
      expect(wrapper.classes()).toContain(client)
    }
  })

  it('names the server e-Boekhouden MCP where the client allows it', () => {
    expect(SERVER_NAMES.claude).toBe('e-Boekhouden MCP')
    expect(SERVER_NAMES.codex).toMatch(/^[a-z0-9_-]+$/)
    expect(SERVER_NAMES.claudeCode).toMatch(/^[a-z0-9_-]+$/)
    expect(usesBridge('codex')).toBe(true)
    expect(usesBridge('cursor')).toBe(false)
  })
})

describe('instellingen', () => {
  it('offers Openen bij inloggen and Toon in Dock as switches', async () => {
    const wrapper = await mountAt('/instellingen')
    const switches = wrapper.findAll('[role="switch"]').map((s) => s.attributes('aria-label'))
    expect(switches).toEqual(['Openen bij inloggen', 'Toon in Dock'])
    // On by default (Sherin, 2026-11-10).
    expect(wrapper.find('[aria-label="Toon in Dock"]').attributes('aria-checked')).toBe('true')
    wrapper.unmount()
  })

  it('explains Diagnose without a page and names the computer', async () => {
    const wrapper = await mountAt('/instellingen')
    expect(wrapper.text()).toContain('wat EB MCP naar e-Boekhouden stuurde en terugkreeg')
    expect(wrapper.text()).not.toContain('op de pagina')
    expect(wrapper.text()).toContain('Systeem volgt de instelling van je Mac.')
    wrapper.unmount()
  })
})

describe('sheets and alerts', () => {
  const sheetOpen = ref(false)
  const alertOpen = ref(false)
  const Host = defineComponent({
    setup: () => () =>
      h('div', [
        h('button', { id: 'opener', type: 'button', onClick: () => (sheetOpen.value = true) }, 'Open'),
        sheetOpen.value
          ? h(Sheet, { title: 'Blad', onClose: () => (sheetOpen.value = false) }, {
            default: () => h('input', { id: 'first' }),
            footer: () => [
              h('button', { id: 'ask', type: 'button', class: 'btn', onClick: () => (alertOpen.value = true) }, 'Vraag'),
              h('button', { id: 'last', type: 'button', class: 'btn primary' }, 'Opslaan'),
            ],
          })
          : null,
        alertOpen.value
          ? h(Alert, { title: 'Zeker?', tone: 'danger', onClose: () => (alertOpen.value = false) }, {
            buttons: () => [
              h('button', { id: 'cancel', type: 'button', class: 'btn' }, 'Annuleren'),
              h('button', { id: 'remove', type: 'button', class: 'btn danger-fill' }, 'Verwijderen'),
            ],
          })
          : null,
      ]),
  })

  const focused = () => (document.activeElement as HTMLElement | null)?.id
  const press = (key: string, shiftKey = false) =>
    (document.activeElement ?? document.body).dispatchEvent(new KeyboardEvent('keydown', { key, shiftKey, bubbles: true, cancelable: true }))

  it('keeps Tab inside the top dialog, closes only the top one with Esc and gives the focus back', async () => {
    const wrapper = mount(Host, { attachTo: document.body })
    const opener = wrapper.find('#opener').element as HTMLButtonElement
    opener.focus()
    await wrapper.find('#opener').trigger('click')
    await flushPromises()
    expect(focused()).toBe('first')

    press('Tab', true)
    expect(focused()).toBe('last')
    press('Tab')
    expect(focused()).toBe('first')

    ;(wrapper.find('#ask').element as HTMLButtonElement).focus()
    await wrapper.find('#ask').trigger('click')
    await flushPromises()
    // A destructive alert focuses Annuleren, not the red button.
    expect(focused()).toBe('cancel')
    press('Tab', true)
    expect(focused()).toBe('remove')
    press('Tab')
    expect(focused()).toBe('cancel')

    press('Escape')
    await flushPromises()
    expect(alertOpen.value).toBe(false)
    expect(sheetOpen.value).toBe(true)
    expect(focused()).toBe('ask')

    press('Escape')
    await flushPromises()
    expect(sheetOpen.value).toBe(false)
    expect(focused()).toBe('opener')
    wrapper.unmount()
  })
})
