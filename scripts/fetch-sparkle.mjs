// Fetches Sparkle, the macOS update framework, into .sparkle: the framework the app bundles
// (src-tauri/tauri.macos.conf.json) and the tools the release signs its updates with. Pinned and checked against its SHA-256,
// so a build never runs code it did not expect. Does nothing on other systems, which update through Tauri instead.
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const VERSION = '2.10.0'
const SHA256 = 'c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c'
const URL = `https://github.com/sparkle-project/Sparkle/releases/download/${VERSION}/Sparkle-${VERSION}.tar.xz`

if (process.platform !== 'darwin') {
  process.exit(0)
}

const target = join(dirname(fileURLToPath(import.meta.url)), '..', '.sparkle')
const stamp = join(target, 'VERSION')
if (existsSync(stamp) && readFileSync(stamp, 'utf8').trim() === VERSION && existsSync(join(target, 'Sparkle.framework'))) {
  process.exit(0)
}

const response = await fetch(URL)
if (!response.ok) {
  throw new Error(`Downloading Sparkle ${VERSION} failed: HTTP ${response.status}`)
}
const archive = Buffer.from(await response.arrayBuffer())
const actual = createHash('sha256').update(archive).digest('hex')
if (actual !== SHA256) {
  throw new Error(`Sparkle ${VERSION} has SHA-256 ${actual}, expected ${SHA256}`)
}

rmSync(target, { recursive: true, force: true })
mkdirSync(target, { recursive: true })
const file = join(target, `Sparkle-${VERSION}.tar.xz`)
writeFileSync(file, archive)
// The system tar keeps the framework's symlinks, which a bundle needs.
execFileSync('tar', ['-xJf', file, '-C', target, 'Sparkle.framework', 'bin/sign_update', 'LICENSE'])
rmSync(file)
writeFileSync(stamp, `${VERSION}\n`)
console.log(`Sparkle ${VERSION} is in ${target}`)
