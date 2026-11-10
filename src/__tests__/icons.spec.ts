import { describe, expect, it } from 'vitest'

// Every icon a page asks for must exist in Icon.vue; a missing name draws an empty square without any error.
const sources = import.meta.glob(['../**/*.vue', '../**/*.ts', '!../__tests__/**'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>
const iconFile = sources['../components/Icon.vue'] ?? ''

describe('icons', () => {
  it('has a path for every icon name used in the app', () => {
    const known = new Set([...iconFile.matchAll(/^ {2}'?([a-z-]+)'?: '</gm)].map((m) => m[1]))
    const used = new Set<string>()
    for (const [file, text = ''] of Object.entries(sources)) {
      if (file.endsWith('Icon.vue')) continue
      for (const m of text.matchAll(/<Icon\s[^>]*?(?<![:\w-])name="([a-z-]+)"/g)) m[1] && used.add(m[1])
      for (const m of text.matchAll(/\bicon: '([a-z-]+)'/g)) m[1] && used.add(m[1])
    }
    expect(used.size).toBeGreaterThan(10)
    expect([...used].filter((name) => !known.has(name))).toEqual([])
  })
})
