import { describe, expect, it } from 'vitest'
import {
  duration, isError, phaseOf, phases, segments, size, statusLabel, timeline, waitNote, waitSteps, workMs, workOffset,
  type EbEntry, type McpEntry,
} from '../lib/log'

const mcp = (id: number, calls: number[], outcome: McpEntry['outcome'] = 'ok'): McpEntry => ({
  kind: 'mcp', id, at: id * 1000, client: 'Claude', tool: 'relaties', args: {}, account: 'Studio Noord',
  outcome, durationMs: 400, bytes: 10, error: null, calls, version: '0.2.0', waitStartedAt: null, waitEndedAt: null, waitedMs: 0,
})
const eb = (id: number, parent: number | null, status = 200): EbEntry => ({
  kind: 'eb', id, at: id * 1000, parent, client: 'Claude', account: 'Studio Noord', method: 'GET', path: 'relatie/gridtable',
  query: [], status, durationMs: 300, bytes: 10, rows: 2, error: status >= 400 ? 'fout' : null, headers: [], truncated: false,
  version: '',
})

describe('request log helpers', () => {
  it('puts e-Boekhouden calls under the MCP call that caused them, in time order', () => {
    const rows = timeline([mcp(10, [12, 11]), eb(12, 10), eb(11, 10), eb(5, null)])
    expect(rows.map((r) => r.entry.id)).toEqual([10, 5])
    expect(rows[0]!.children.map((c) => c.id)).toEqual([11, 12])
  })

  it('keeps a call on its own when its MCP call left the log', () => {
    expect(timeline([eb(3, 1)]).map((r) => r.entry.id)).toEqual([3])
  })

  it('labels and flags outcomes', () => {
    expect(statusLabel(mcp(1, [], 'refused'))).toBe('Geweigerd')
    expect(isError(mcp(1, [], 'refused'))).toBe(true)
    expect(statusLabel(eb(1, null, 500))).toBe('500')
    expect(isError(eb(1, null, 500))).toBe(true)
    expect(isError(eb(1, null, 200))).toBe(false)
  })

  it('formats durations and sizes the Dutch way', () => {
    expect(duration(388)).toBe('388 ms')
    expect(duration(1240)).toBe('1,24 s')
    expect(size(3686)).toBe('3,6 kB')
    expect(size(512)).toBe('512 B')
  })
})

/** A change that waited: 400 ms preparing, then the wait, then 600 ms making it, as the log records it. */
const waitedCall = (over: Partial<McpEntry> = {}): McpEntry => ({
  ...mcp(20, [21, 22, 23]), tool: 'mutatie_toevoegen', at: 100_000, proposal: 7,
  waitStartedAt: 100_400, waitEndedAt: 147_700, waitedMs: 47_300, durationMs: 48_300, ...over,
})
const call = (id: number, at: number, method = 'GET', path = 'grootboekrekening/gridtable'): EbEntry => ({ ...eb(id, 20), at, method, path, durationMs: 150 })

describe('the wait for approval', () => {
  it('splits a call that waited into preparing, waiting and making the change', () => {
    expect(phases(waitedCall())).toEqual({ prepMs: 400, waitMs: 47_300, execMs: 600, totalMs: 48_300, workMs: 1_000, waiting: false })
    expect(workMs(waitedCall())).toBe(1_000)
    expect(waitNote(waitedCall())).toBe('+ 47,3 s gewacht')
  })

  it('counts a wait that still runs up to now, and the work as the preparing so far', () => {
    const entry = waitedCall({ outcome: 'waiting', waitEndedAt: null, waitedMs: 0, durationMs: 0 })
    const now = 100_400 + 4 * 60_000 + 12_000
    expect(phases(entry, now)).toEqual({ prepMs: 400, waitMs: 252_000, execMs: 0, totalMs: 252_400, workMs: 400, waiting: true })
    expect(workMs(entry, now)).toBe(400)
    expect(waitNote(entry, now)).toBe('wacht al 4 min')
    expect(waitNote(entry, 100_400 + 12_900)).toBe('wacht al 12 s')
  })

  it('has nothing after the wait of a refused call', () => {
    const entry = waitedCall({ outcome: 'refused', waitEndedAt: 112_400, waitedMs: 12_000, durationMs: 12_400, error: 'De gebruiker heeft dit geweigerd in EB MCP. Er is niets gewijzigd.' })
    expect(phases(entry)).toMatchObject({ prepMs: 400, waitMs: 12_000, execMs: 0, workMs: 400 })
    expect(waitNote(entry)).toBe('+ 12,0 s gewacht')
    expect(segments(entry, 2_000).map((s) => s.phase)).toEqual(['prep', 'wait'])
  })

  it('leaves a call that never waited as it was', () => {
    const plain = mcp(1, [])
    expect(phases(plain)).toBeNull()
    expect(workMs(plain)).toBe(400)
    expect(waitNote(plain)).toBe('')
    expect(segments(plain, 2_000)).toEqual([])
    expect(workMs(eb(2, null))).toBe(300)
  })

  it('sizes the bar on the work scale and lets the wait fill the rest of the track', () => {
    const parts = segments(waitedCall(), 2_000)
    expect(parts.map((s) => s.phase)).toEqual(['prep', 'wait', 'exec'])
    expect(parts.map((s) => Math.round(s.width))).toEqual([20, 50, 30])
    // A short wait takes its own share, not the rest of the track.
    const short = segments(waitedCall({ waitEndedAt: 100_600, waitedMs: 200, durationMs: 1_200 }), 10_000)
    expect(short.map((s) => Math.round(s.width))).toEqual([4, 4, 6])
    // Work too small to see keeps 4% between preparing and making the change.
    const tiny = segments(waitedCall(), 100_000)
    expect(tiny[0]!.width + tiny[2]!.width).toBeCloseTo(4)
    expect(tiny[1]!.width).toBeCloseTo(47.3)
    // A wait that would run past the track ends at its end.
    const long = segments(waitedCall({ waitEndedAt: 700_400, waitedMs: 600_000, durationMs: 601_000 }), 100_000)
    expect(long.reduce((n, s) => n + s.width, 0)).toBeCloseTo(100)
  })

  it('puts each e-Boekhouden call in the phase it started in', () => {
    const entry = waitedCall()
    expect(phaseOf(call(21, 100_100), entry)).toBe('prep')
    expect(phaseOf(call(22, 100_400), entry)).toBe('prep')
    expect(phaseOf(call(23, 147_800, 'POST', 'mutatie'), entry)).toBe('exec')
    // The waterfall leaves the wait out.
    expect(workOffset(entry, 100_100)).toBe(100)
    expect(workOffset(entry, 147_800)).toBe(500)
  })

  it('lists the steps of an approved call with its calls under the phase they belong to', () => {
    const calls = [call(23, 147_800, 'POST', 'mutatie'), call(21, 100_100), call(22, 100_250, 'GET', 'btwinfo/all')]
    const steps = waitSteps(waitedCall(), calls, { decision: 'approved', bulk: false })
    expect(steps.map((s) => s.key)).toEqual(['received', 'prep', 'answered', 'wait', 'approved', 'exec', 'done'])
    expect(steps.map((s) => s.title)).toEqual(['Aanroep ontvangen', 'Voorbereiden', 'Antwoord aan Claude', 'Wacht op akkoord', 'Goedgekeurd', 'Uitvoeren', 'Klaar'])
    expect(steps[1]!.value).toBe('400 ms')
    expect(steps[1]!.calls!.map((c) => c.id)).toEqual([21, 22])
    expect(steps[2]!.note).toBe('Status wacht, voorstel 7')
    expect(steps[3]).toMatchObject({ value: '47,3 s', chip: 'niet meegeteld', note: undefined })
    expect(steps[4]!.value).not.toContain('Alles goedkeuren')
    expect(steps[5]).toMatchObject({ value: '600 ms' })
    expect(steps[5]!.calls!.map((c) => c.id)).toEqual([23])
    expect(steps[0]!.value).toMatch(/^\d{2}:\d{2}:\d{2}\.\d{3}$/)
  })

  it('names a bulk approval and a failure', () => {
    const failed = waitedCall({ outcome: 'error', error: 'Niet ingelogd bij Studio Noord.' })
    const steps = waitSteps(failed, [], { decision: 'approved', bulk: true })
    expect(steps.find((s) => s.key === 'approved')!.value).toMatch(/ · Alles goedkeuren$/)
    expect(steps.at(-1)).toMatchObject({ key: 'failed', title: 'Mislukt', note: 'Niet ingelogd bij Studio Noord.', tone: 'bad' })
  })

  it('ends at the refusal or the lapse, with the reason', () => {
    const refused = waitedCall({ outcome: 'refused', waitEndedAt: 112_400, waitedMs: 12_000, durationMs: 12_400, error: 'De gebruiker heeft dit geweigerd in EB MCP. Er is niets gewijzigd.' })
    const steps = waitSteps(refused, [], null)
    expect(steps.map((s) => s.key)).toEqual(['received', 'prep', 'answered', 'wait', 'refused'])
    expect(steps.at(-1)).toMatchObject({ title: 'Geweigerd', note: 'De gebruiker heeft dit geweigerd in EB MCP. Er is niets gewijzigd.' })
    expect(waitSteps(refused, [], { decision: 'refused', bulk: true }).at(-1)!.value).toMatch(/ · Alles weigeren$/)
    const lapsed = { ...refused, error: 'EB MCP is opnieuw gestart voordat de gebruiker antwoordde; het voorstel is vervallen en er is niets gewijzigd.' }
    expect(waitSteps(lapsed, [], null).at(-1)).toMatchObject({ key: 'lapsed', title: 'Vervallen' })
    expect(waitSteps(refused, [], { decision: 'lapsed', bulk: false }).at(-1)!.title).toBe('Vervallen')
  })

  it('stops at the wait while it still waits, and has no steps without one', () => {
    const entry = waitedCall({ outcome: 'waiting', waitEndedAt: null, waitedMs: 0, durationMs: 0 })
    const steps = waitSteps(entry, [], null, 100_400 + 4 * 60_000)
    expect(steps.map((s) => s.key)).toEqual(['received', 'prep', 'answered', 'wait'])
    expect(steps.at(-1)).toMatchObject({ value: '4 min', note: 'wacht nog' })
    expect(waitSteps(mcp(1, []), [], null)).toEqual([])
  })
})
