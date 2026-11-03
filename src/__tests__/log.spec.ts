import { describe, expect, it } from 'vitest'
import { duration, isError, size, statusLabel, timeline, type EbEntry, type McpEntry } from '../lib/log'

const mcp = (id: number, calls: number[], outcome: McpEntry['outcome'] = 'ok'): McpEntry => ({
  kind: 'mcp', id, at: id * 1000, client: 'Claude', tool: 'relaties', args: {}, account: 'Studio Noord',
  outcome, durationMs: 400, bytes: 10, error: null, calls,
})
const eb = (id: number, parent: number | null, status = 200): EbEntry => ({
  kind: 'eb', id, at: id * 1000, parent, client: 'Claude', account: 'Studio Noord', method: 'GET', path: 'relatie/gridtable',
  query: [], status, durationMs: 300, bytes: 10, rows: 2, error: status >= 400 ? 'fout' : null, headers: [], truncated: false,
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
