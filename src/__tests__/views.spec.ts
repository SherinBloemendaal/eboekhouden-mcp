import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import { router } from '../router'
import App from '../App.vue'

async function mountAt(path: string) {
  router.push(path)
  await router.isReady()
  const wrapper = mount(App, { global: { plugins: [router] } })
  await flushPromises()
  return wrapper
}

describe('views', () => {
  it('shows the four pages in the sidebar, Dashboard first', async () => {
    const wrapper = await mountAt('/dashboard')
    const labels = wrapper.findAll('nav a').map((a) => a.text())
    expect(labels).toEqual(['Dashboard', 'Accounts', 'Verzoeken', 'Koppelen'])
    expect(wrapper.find('header h1').text()).toBe('Dashboard')
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
