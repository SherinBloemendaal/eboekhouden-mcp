import { describe, expect, it } from 'vitest'
import { initials, isRunning, outcomeLabel, sessionAge, since } from '../lib/accounts'

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
    expect(since(now - 120 * 60_000, now)).toBe('2 u')
    expect(since(now - 121 * 60_000, now)).toBe('2 u 1 min')
  })

  it('says how long ago a session signed in, without a zero minute part', () => {
    const now = Date.UTC(2026, 10, 3, 12, 0)
    expect(sessionAge(now, now)).toBe('Zojuist ingelogd')
    expect(sessionAge(now - 59_999, now)).toBe('Zojuist ingelogd')
    expect(sessionAge(now - 60_000, now)).toBe('1 min ingelogd')
    expect(sessionAge(now - 12 * 60_000, now)).toBe('12 min ingelogd')
    expect(sessionAge(now - 120 * 60_000, now)).toBe('2 u ingelogd')
    expect(sessionAge(now - 150 * 60_000, now)).toBe('2 u 30 min ingelogd')
    // A clock that went back reads as just now, never as a negative age.
    expect(sessionAge(now + 5 * 60_000, now)).toBe('Zojuist ingelogd')
  })

  it('names the outcome of a sign-in in Dutch', () => {
    expect(outcomeLabel('Done')).toEqual({ text: 'Gelukt', tone: 'ok' })
    expect(outcomeLabel('Done: Studio Noord')).toEqual({ text: 'Gelukt', tone: 'ok' })
    expect(outcomeLabel('Failed: Gebruikersnaam of wachtwoord klopt niet')).toEqual({ text: 'Mislukt', tone: 'bad' })
    expect(outcomeLabel('Unknown: Onverwachte pagina')).toEqual({ text: 'Onbekende fout', tone: 'bad' })
    expect(outcomeLabel('Cancelled')).toEqual({ text: 'Geannuleerd', tone: 'off' })
    expect(outcomeLabel(null)).toEqual({ text: 'Loopt', tone: 'run' })
    expect(outcomeLabel('Iets anders')).toEqual({ text: 'Onbekende fout', tone: 'bad' })
  })

  it('knows which steps are still running', () => {
    expect(isRunning('submit')).toBe(true)
    expect(isRunning('code')).toBe(true)
    expect(isRunning('done')).toBe(false)
    expect(isRunning(undefined)).toBe(false)
  })
})
