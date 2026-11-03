import { describe, expect, it } from 'vitest'
import { initials, isRunning, since } from '../lib/accounts'

describe('account helpers', () => {
  it('makes initials from the label', () => {
    expect(initials('Studio Noord')).toBe('SN')
    expect(initials('Demo')).toBe('DE')
    expect(initials('  atelier  zuid ')).toBe('AZ')
  })

  it('says how long a session runs', () => {
    const now = Date.UTC(2026, 10, 3, 12, 0)
    expect(since(now - 20_000, now)).toBe('net')
    expect(since(now - 12 * 60_000, now)).toBe('12 min')
    expect(since(now - 150 * 60_000, now)).toBe('2 u 30 min')
  })

  it('knows which steps are still running', () => {
    expect(isRunning('submit')).toBe(true)
    expect(isRunning('code')).toBe(true)
    expect(isRunning('done')).toBe(false)
    expect(isRunning(undefined)).toBe(false)
  })
})
