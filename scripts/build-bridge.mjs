// Builds eb-mcp-bridge and puts it where Tauri bundles sidecars:
// src-tauri/binaries/eb-mcp-bridge-<target triple>[.exe]. Tauri copies it next
// to the app's executable, where Koppelen points Claude Desktop to.
//
//   node scripts/build-bridge.mjs                      (this machine, release)
//   node scripts/build-bridge.mjs --target universal-apple-darwin
//   node scripts/build-bridge.mjs --debug              (for npm run tauri dev)
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri')
const args = process.argv.slice(2)
const debug = args.includes('--debug')
const targetIndex = args.indexOf('--target')
// tauri build runs this as its beforeBuildCommand and says which target it builds for.
const target = targetIndex >= 0 ? args[targetIndex + 1] : (process.env.TAURI_ENV_TARGET_TRIPLE ?? null)
const host = execFileSync('rustc', ['-vV'], { encoding: 'utf8' }).match(/^host: (.+)$/m)[1]
const ext = (target ?? host).includes('windows') ? '.exe' : ''
// Cargo's own output folder, which CARGO_TARGET_DIR can move.
const targetDir = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(root, 'target')

function build(triple) {
  const cargo = ['build', '--manifest-path', join(root, 'bridge', 'Cargo.toml')]
  if (!debug) cargo.push('--release')
  if (triple) cargo.push('--target', triple)
  execFileSync('cargo', cargo, { stdio: 'inherit' })
  return join(targetDir, ...(triple ? [triple] : []), debug ? 'debug' : 'release', `eb-mcp-bridge${ext}`)
}

if (debug) {
  // tauri dev also needs the sidecar file to exist; a debug build is enough there.
  const built = build(null)
  mkdirSync(join(root, 'binaries'), { recursive: true })
  copyFileSync(built, join(root, 'binaries', `eb-mcp-bridge-${host}${ext}`))
} else if (target === 'universal-apple-darwin') {
  // tauri build --target universal-apple-darwin compiles the app once per architecture and looks for the sidecar
  // under each of those triples, then bundles the universal one: all three files must exist.
  const arm = build('aarch64-apple-darwin')
  const intel = build('x86_64-apple-darwin')
  mkdirSync(join(root, 'binaries'), { recursive: true })
  copyFileSync(arm, join(root, 'binaries', 'eb-mcp-bridge-aarch64-apple-darwin'))
  copyFileSync(intel, join(root, 'binaries', 'eb-mcp-bridge-x86_64-apple-darwin'))
  execFileSync('lipo', ['-create', '-output', join(root, 'binaries', 'eb-mcp-bridge-universal-apple-darwin'), arm, intel], { stdio: 'inherit' })
} else {
  const built = build(target)
  mkdirSync(join(root, 'binaries'), { recursive: true })
  copyFileSync(built, join(root, 'binaries', `eb-mcp-bridge-${target ?? host}${ext}`))
}
