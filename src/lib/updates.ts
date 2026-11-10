// Updates, as in Tresso: Sparkle on macOS (its own windows; we only answer its relaunch question), Tauri's updater
// on Windows (an update card in the sidebar). EB MCP's sessions live in memory, so a restart signs everyone out:
// the user picks when.
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { reactive } from 'vue'
import { inApp } from './server'

export type Channel = 'sparkle' | 'tauri' | 'none'

interface TauriUpdate {
  version: string
  body?: string
  downloadAndInstall(onEvent?: (event: { event: string; data?: { contentLength?: number; chunkLength?: number } }) => void): Promise<void>
}

export const updates = reactive({
  channel: 'none' as Channel,
  /** macOS: Sparkle downloaded an update and waits for our answer. */
  restartAsked: false,
  /** Windows: an update the card offers. */
  available: null as { version: string; notes: string } | null,
  progress: null as number | null,
  error: null as string | null,
  restarting: false,
})

let pending: TauriUpdate | null = null
let started = false
const SIX_HOURS = 6 * 60 * 60 * 1000

/** Main window only: listens for Sparkle's question and checks the Tauri feed. */
export async function startUpdates(): Promise<void> {
  if (started || !inApp()) return
  started = true
  updates.channel = await invoke<Channel>('update_channel')
  if (updates.channel === 'sparkle') {
    await listen('update-relaunch-requested', () => void relaunchAsked())
    // A question asked while this page was away (a reload) still waits for its answer.
    if (await invoke<boolean>('update_listening')) void relaunchAsked()
  } else if (updates.channel === 'tauri') {
    await check()
    setInterval(check, SIX_HOURS)
  }
}

/**
 * Sparkle wants to restart into the update the user just chose to install in its own window. Asking again would be
 * the same question twice, so EB MCP restarts right away; it only asks while an AI client still waits on a call.
 */
async function relaunchAsked(): Promise<void> {
  const running = await invoke<number>('running_calls').catch(() => 0)
  if (running > 0) updates.restartAsked = true
  else await restartNow()
}

async function check(): Promise<void> {
  try {
    const { check: checkUpdate } = await import('@tauri-apps/plugin-updater')
    const update = (await checkUpdate()) as TauriUpdate | null
    pending = update
    updates.available = update ? { version: update.version, notes: update.body ?? '' } : null
    updates.error = null
  } catch {
    // Offline or the feed is unreachable: try again at the next check, without bothering anyone.
  }
}

/** "Zoek naar updates…": Sparkle's own window on macOS; a fresh check on Windows. */
export async function checkNow(): Promise<void> {
  if (!inApp()) return
  const channel = updates.channel === 'none' ? await invoke<Channel>('update_channel') : updates.channel
  if (channel === 'sparkle') await invoke('check_for_updates_natively')
  else if (channel === 'tauri') await check()
}

/** Signs everyone out, waits for running calls, then restarts into the update. */
export async function restartNow(): Promise<void> {
  updates.restarting = true
  updates.error = null
  try {
    await invoke('prepare_update_restart', { ready: true })
    if (updates.channel === 'sparkle') {
      updates.restartAsked = false
      await invoke('update_relaunch', { proceed: true })
    } else if (pending) {
      updates.progress = 0
      let total = 0
      let done = 0
      await pending.downloadAndInstall((event) => {
        if (event.event === 'Started') total = event.data?.contentLength ?? 0
        if (event.event === 'Progress') {
          done += event.data?.chunkLength ?? 0
          updates.progress = total ? Math.min(1, done / total) : null
        }
      })
      const { relaunch } = await import('@tauri-apps/plugin-process')
      await relaunch()
    }
  } catch (e) {
    updates.error = String(e)
    await invoke('prepare_update_restart', { ready: false })
  } finally {
    updates.restarting = false
  }
}

/** Not now: Sparkle installs the update when EB MCP quits. */
export async function later(): Promise<void> {
  updates.restartAsked = false
  if (updates.channel === 'sparkle') await invoke('update_relaunch', { proceed: false })
}
