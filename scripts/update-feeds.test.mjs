import { Buffer } from 'node:buffer'
import { describe, expect, it } from 'vitest'
import { appcast, latestJson, parseSparkleSignature, signedVersionOf, versionOf } from './update-feeds.mjs'

const sig = (version) => Buffer.from(`untrusted comment: x\nRW...\ntrusted comment: timestamp:1\tfile:eb-mcp-windows-x64-setup.exe\tversion:${version}\nabc\n`).toString('base64')

describe('update feeds', () => {
  it('reads tags and signed versions', () => {
    expect(versionOf('v0.2.0')).toBe('0.2.0')
    expect(() => versionOf('0.2.0')).toThrow()
    expect(signedVersionOf(sig('0.2.0'))).toBe('0.2.0')
  })

  it('refuses a signature for another version', () => {
    expect(() => latestJson({ tag: 'v0.2.0', notes: '', pubDate: 'x', signatureOf: () => sig('0.1.9') })).toThrow(/not 0.2.0/)
    const feed = latestJson({ tag: 'v0.2.0', notes: 'n', pubDate: 'x', signatureOf: () => sig('0.2.0') })
    expect(Object.keys(feed.platforms)).toEqual(['windows-x86_64', 'windows-x86_64-nsis'])
    expect(feed.platforms['windows-x86_64'].url).toMatch(/releases\/download\/v0\.2\.0\/eb-mcp-windows-x64-setup\.exe$/)
  })

  it('writes one appcast item with the build number and the EdDSA signature', () => {
    const { signature, length } = parseSparkleSignature('sparkle:edSignature="AbC+/=" length="1234"')
    const xml = appcast({ tag: 'v0.2.0', build: '17', file: 'eb-mcp-macos-universal.app.tar.gz', notesHtml: '<p>a]]>b</p>', pubDate: '2026-11-03T12:00:00Z', signature, length })
    expect(xml).toContain('<sparkle:version>17</sparkle:version>')
    expect(xml).toContain('<sparkle:shortVersionString>0.2.0</sparkle:shortVersionString>')
    expect(xml).toContain('sparkle:edSignature="AbC+/="')
    expect(xml).toContain(']]]]><![CDATA[>')
  })
})
