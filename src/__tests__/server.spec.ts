import { describe, expect, it } from 'vitest'
import { inApp, masked, serverInfo } from '../lib/server'

describe('server helpers', () => {
  it('shows only the tail of the token', () => {
    expect(masked('3f9a')).toBe('••••••••••••3f9a')
    expect(masked('')).toBe('••••••••••••')
  })

  it('says there is no server outside the app', async () => {
    expect(inApp()).toBe(false)
    const info = await serverInfo()
    expect(info.running).toBe(false)
    expect(info.url).toBe('http://127.0.0.1:47821/mcp')
  })
})
