// Prepares a release build's update configuration from the repository's settings, so the source holds no keys and
// no feed address of a repository that may move. Run by the release workflow before `tauri build`.
//
//   UPDATE_REPO            owner/name of the GitHub repository whose releases carry the feeds (GITHUB_REPOSITORY)
//   SPARKLE_PUBLIC_KEY     Sparkle's EdDSA public key (macOS)
//   TAURI_UPDATER_PUBKEY   the Tauri updater's minisign public key (Windows)
//
// Writes src-tauri/Info.plist (merged into the macOS app by Tauri) and prints the JSON to pass as
// `tauri build --config`, with the version and, on macOS, the build number. A build without these values (any
// development build) carries no updater at all, and the app then never offers an update.
//
// Usage: node scripts/release-config.mjs <version> <build>
import { writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const [version, build] = process.argv.slice(2)
const repo = process.env.UPDATE_REPO ?? process.env.GITHUB_REPOSITORY
const sparkleKey = process.env.SPARKLE_PUBLIC_KEY
const tauriKey = process.env.TAURI_UPDATER_PUBKEY

if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(version)) throw new Error(`Not a version: ${version}`)
if (!build || !/^[1-9]\d*$/.test(build)) throw new Error(`Not a build number: ${build}`)
if (!repo || !/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new Error('UPDATE_REPO (owner/name) is missing')
if (!sparkleKey || !/^[A-Za-z0-9+/]{43}=$/.test(sparkleKey)) throw new Error('SPARKLE_PUBLIC_KEY is missing or not an EdDSA public key')
if (!tauriKey) throw new Error('TAURI_UPDATER_PUBKEY is missing')

const feeds = `https://github.com/${repo}/releases/latest/download`
const plist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by scripts/release-config.mjs for a release build: Sparkle's feed and the key its updates are signed with. -->
<plist version="1.0">
<dict>
  <key>SUFeedURL</key>
  <string>${feeds}/appcast.xml</string>
  <key>SUPublicEDKey</key>
  <string>${sparkleKey}</string>
  <key>SUEnableAutomaticChecks</key>
  <true/>
  <key>SUScheduledCheckInterval</key>
  <integer>21600</integer>
  <key>SUVerifyUpdateBeforeExtraction</key>
  <true/>
</dict>
</plist>
`
writeFileSync(join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'Info.plist'), plist)

const config = {
  version,
  bundle: { macOS: { bundleVersion: build } },
  plugins: {
    updater: {
      pubkey: tauriKey,
      endpoints: [`${feeds}/latest.json`],
      requireSignedVersion: true,
      windows: { installMode: 'passive' },
    },
  },
}
process.stdout.write(JSON.stringify(config))
