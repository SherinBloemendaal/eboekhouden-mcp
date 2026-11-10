import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import { router } from '../router'
import App from '../App.vue'
import ClientIcon from '../components/ClientIcon.vue'
import { CLIENT_NAMES, SERVER_NAMES, usesBridge, type Client } from '../lib/connect'

async function mountAt(path: string) {
  router.push(path)
  await router.isReady()
  const wrapper = mount(App, { global: { plugins: [router] } })
  await flushPromises()
  return wrapper
}

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

  it('shows the server address and a masked token on Koppelen', async () => {
    const wrapper = await mountAt('/koppelen')
    expect(wrapper.text()).toContain('http://127.0.0.1:47821/mcp')
    expect(wrapper.text()).toContain('••••')
  })
})

describe('accounts page', () => {
  it('shows the empty state with a way to add an account', async () => {
    const wrapper = await mountAt('/accounts')
    expect(wrapper.text()).toContain('Nog geen accounts')
    expect(wrapper.find('button.primary').text()).toContain('Account toevoegen')
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
})
