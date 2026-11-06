// Writes the update feeds of a release from the signed files in a folder:
//   latest.json   the Tauri updater feed (the Windows setup)
//   appcast.xml   the Sparkle feed (macOS, one universal app)
// The release workflow runs it once the release is a draft with its files; the apps read the feeds from
// https://github.com/<repository>/releases/latest/download/<feed> (scripts/release-config.mjs), so they see a release
// only once both are there. Ported from Tresso's scripts/update-feeds.mjs.
//
// Usage: node scripts/update-feeds.mjs <dist> <tag> <build> <notes.md> <notes.html>
//   <dist> holds the release files, their Tauri updater signatures (<file>.sig) and the Sparkle signature
//   (<file>.sparkle, the attribute line sign_update prints).
//   <build> is the macOS build number (CFBundleVersion), the workflow's run number.

import { Buffer } from 'node:buffer'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import process from 'node:process'
import { pathToFileURL } from 'node:url'

/** The repository whose releases carry the files: the workflow's own. */
const REPO = process.env.UPDATE_REPO ?? process.env.GITHUB_REPOSITORY ?? 'SherinBloemendaal/eboekhouden-mcp'

/**
 * Tauri updater platforms and the release file each installs from. The updater asks for `<os>-<arch>-<installer>`
 * first and falls back to `<os>-<arch>`; both name the same file. Only the setup updates in the app: an .msi install
 * updates by installing the new package (src-tauri/src/updates.rs).
 */
export const TAURI_PLATFORMS = [
  ['windows-x86_64', 'eb-mcp-windows-x64-setup.exe'],
  ['windows-x86_64-nsis', 'eb-mcp-windows-x64-setup.exe'],
]

/** The Sparkle feed (SUFeedURL, scripts/release-config.mjs) and the archive it offers. */
export const SPARKLE_FEED = ['appcast.xml', 'eb-mcp-macos-universal.app.tar.gz']

/** The oldest macOS the app runs on (bundle.macOS.minimumSystemVersion in src-tauri/tauri.conf.json). */
export const MINIMUM_MACOS = '13.0'

/** `v1.2.3` or `v1.2.3-beta.1` to the version without the v; throws for anything else. */
export function versionOf(tag) {
  const match = /^v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)$/.exec(tag)
  if (!match) {
    throw new Error(`Tag "${tag}" is not of the form vMAJOR.MINOR.PATCH[-PRE]`)
  }
  return match[1]
}

/** A macOS build number (CFBundleVersion, `sparkle:version`): a positive integer; throws otherwise. */
export function buildOf(build) {
  if (!/^[1-9]\d*$/.test(String(build))) {
    throw new Error(`Build number "${build}" is not a positive integer`)
  }
  return String(build)
}

/**
 * The version a Tauri updater signature was signed for, from its trusted comment
 * (`timestamp:...\tfile:...\tversion:1.2.3`), or null. `<file>.sig` holds the minisign signature file in base64.
 */
export function signedVersionOf(signature) {
  const text = Buffer.from(signature, 'base64').toString('utf8')
  const comment = /^trusted comment: (.*)$/m.exec(text)?.[1]
  if (!comment) {
    return null
  }
  for (const field of comment.split('\t')) {
    if (field.startsWith('version:')) {
      return field.slice('version:'.length).trim()
    }
  }
  return null
}

function fileUrl(tag, file) {
  return `https://github.com/${REPO}/releases/download/${tag}/${file}`
}

/**
 * The Tauri updater feed. Every platform needs a signature, and every signature must be bound to this version: the
 * app sets `requireSignedVersion`, so it refuses any other, and an older signed file cannot pass as this release.
 */
export function latestJson({ tag, notes, pubDate, signatureOf }) {
  const version = versionOf(tag)
  const platforms = {}
  for (const [platform, file] of TAURI_PLATFORMS) {
    const signature = signatureOf(file)
    if (!signature) {
      throw new Error(`No updater signature for ${platform}`)
    }
    const signed = signedVersionOf(signature)
    if (signed !== version) {
      throw new Error(`The signature of ${file} is for version ${signed ?? '(none)'}, not ${version}`)
    }
    platforms[platform] = { signature, url: fileUrl(tag, file) }
  }
  return { version, notes, pub_date: pubDate, platforms }
}

/** Escapes text for an XML attribute or element. */
export function xmlEscape(text) {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

/** Wraps HTML in CDATA; a literal `]]>` inside is split across two sections. */
export function cdata(html) {
  return `<![CDATA[${html.replaceAll(']]>', ']]]]><![CDATA[>')}]]>`
}

/** Reads `sparkle:edSignature="…" length="…"` as printed by sign_update. */
export function parseSparkleSignature(line) {
  const signature = /sparkle:edSignature="([A-Za-z0-9+/=]+)"/.exec(line)?.[1]
  const length = /length="(\d+)"/.exec(line)?.[1]
  if (!signature || !length) {
    throw new Error(`Not a sign_update line: ${line}`)
  }
  return { signature, length }
}

/**
 * A Sparkle appcast with the one release. Sparkle orders releases by `sparkle:version`, the build number
 * (CFBundleVersion), since it cannot order 0.4.0-beta.1 before 0.4.0; `sparkle:shortVersionString` is the version
 * people see.
 */
export function appcast({ tag, build, file, notesHtml, pubDate, signature, length }) {
  const version = versionOf(tag)
  const bundleVersion = buildOf(build)
  return `<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>EB MCP</title>
    <link>https://github.com/${REPO}</link>
    <item>
      <title>EB MCP ${xmlEscape(version)}</title>
      <pubDate>${xmlEscape(new Date(pubDate).toUTCString())}</pubDate>
      <sparkle:version>${xmlEscape(bundleVersion)}</sparkle:version>
      <sparkle:shortVersionString>${xmlEscape(version)}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>${MINIMUM_MACOS}</sparkle:minimumSystemVersion>
      <sparkle:fullReleaseNotesLink>https://github.com/${REPO}/releases/tag/${xmlEscape(tag)}</sparkle:fullReleaseNotesLink>
      <description>${cdata(notesHtml)}</description>
      <enclosure url="${xmlEscape(fileUrl(tag, file))}" sparkle:edSignature="${xmlEscape(signature)}" length="${xmlEscape(length)}" type="application/octet-stream"/>
    </item>
  </channel>
</rss>
`
}

function main([dist, tag, build, notesFile, notesHtmlFile]) {
  if (!dist || !tag || !build || !notesFile || !notesHtmlFile) {
    throw new Error('Usage: node scripts/update-feeds.mjs <dist> <tag> <build> <notes.md> <notes.html>')
  }
  const notes = readFileSync(notesFile, 'utf8').trim()
  const notesHtml = readFileSync(notesHtmlFile, 'utf8').trim()
  const pubDate = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z')
  const signatureOf = (file) => {
    const path = join(dist, `${file}.sig`)
    return existsSync(path) ? readFileSync(path, 'utf8').trim() : null
  }
  // Windows ships after the first Mac release: without its setup there is no Tauri feed yet.
  if (TAURI_PLATFORMS.some(([, file]) => existsSync(join(dist, file)))) {
    writeFileSync(join(dist, 'latest.json'), `${JSON.stringify(latestJson({ tag, notes, pubDate, signatureOf }), null, 2)}\n`)
  }
  const [feed, file] = SPARKLE_FEED
  const sparkle = join(dist, `${file}.sparkle`)
  if (!existsSync(sparkle)) {
    throw new Error(`No Sparkle signature for ${file}`)
  }
  const { signature, length } = parseSparkleSignature(readFileSync(sparkle, 'utf8'))
  writeFileSync(join(dist, feed), appcast({ tag, build, file, notesHtml, pubDate, signature, length }))
  console.log(`Wrote latest.json and ${feed} for ${tag}`)
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  try {
    main(process.argv.slice(2))
  } catch (error) {
    console.error(error.message)
    process.exit(1)
  }
}
