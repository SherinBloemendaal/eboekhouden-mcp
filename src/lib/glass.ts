import { invoke } from '@tauri-apps/api/core'

/**
 * Real macOS 26 Liquid Glass behind the main window's sidebar card (src-tauri/src/glass.rs). The page reports where
 * the card is; the app puts an NSGlassEffectView there, under the transparent webview. Only after the app answers
 * that glass is there does the card drop its own CSS glass (`onSupported(true)`); an unsupported answer (macOS before
 * 26, Windows, the kill switch) or a failure leaves today's look.
 */

/** One glass shape in CSS pixels from the top-left corner of the page, as `getBoundingClientRect` gives it. */
export interface GlassRect {
  id: string
  x: number
  y: number
  width: number
  height: number
  radius: number
}

export interface GlassAnswer {
  supported: boolean
  shapes: number
}

/** The card's corners, concentric with the window's (tokens in AppShell.vue). */
export const CARD_RADIUS = 18
export const CARD_ID = 'sidebar'
/** How long the sidebar's open and close animation runs (220 ms in AppShell.vue), with a little to spare. */
export const COLLAPSE_FOLLOW_MS = 300

/**
 * Asks the app for exactly these shapes. `viewportWidth` (the page's innerWidth) lets the app turn CSS pixels into
 * points at any page zoom: it compares it with the window's width in points, so devicePixelRatio, which also carries
 * the screen's backing scale, plays no part.
 */
export const setGlass = (rects: GlassRect[], viewportWidth: number): Promise<GlassAnswer> =>
  invoke<GlassAnswer>('set_glass', { viewportWidth, rects })

/** The card as a glass shape, or none when it has no size (collapsed, or not laid out). */
export function cardShape(card: HTMLElement | null): GlassRect[] {
  if (!card) return []
  const box = card.getBoundingClientRect()
  if (!(box.width >= 1 && box.height >= 1)) return []
  return [{ id: CARD_ID, x: box.left, y: box.top, width: box.width, height: box.height, radius: CARD_RADIUS }]
}

export interface GlassSyncOptions {
  /** The sidebar card. */
  card: () => HTMLElement | null
  /** Whether the sidebar is collapsed; once it has settled there, no shape is reported at all. */
  collapsed: () => boolean
  /** Called with true after a supported answer, with false after an unsupported one or a failure. */
  onSupported: (on: boolean) => void
  /** For tests: the call to the app, the frame scheduler and the clock. */
  send?: (rects: GlassRect[], viewportWidth: number) => Promise<GlassAnswer>
  frame?: (callback: () => void) => unknown
  now?: () => number
}

export interface GlassSync {
  /** Reports the card's place now, when it changed. */
  sync(): void
  /** Reports it every animation frame for the next `ms` milliseconds (the card slides or is dragged), then once more. */
  follow(ms: number): void
  /** Removes the shape and stops. */
  stop(): void
}

/**
 * Keeps the app's glass on the card. One call is in flight at a time: what changes meanwhile is sent when it returns,
 * so the last place always arrives and calls never overtake each other. An unsupported answer ends it for good; after
 * a failure the next change tries again.
 */
export function createGlassSync(options: GlassSyncOptions): GlassSync {
  const send = options.send ?? setGlass
  const frame = options.frame ?? ((callback: () => void) => requestAnimationFrame(callback))
  const now = options.now ?? (() => performance.now())
  let last = ''
  let busy = false
  let again = false
  let stopped = false
  let unsupported = false
  let until = 0
  let ticking = false

  const following = () => now() < until

  function shapes(): GlassRect[] {
    // Collapsed and settled: no shape, so the app removes the view instead of keeping one of no width.
    if (options.collapsed() && !following()) return []
    return cardShape(options.card())
  }

  function sync(): void {
    if (stopped || unsupported) return
    if (busy) {
      again = true
      return
    }
    const rects = shapes()
    const viewportWidth = window.innerWidth
    // The window's size is part of it: AppKit counts y from the bottom, so a taller window moves the same shape.
    const key = JSON.stringify([rects, viewportWidth, window.innerHeight])
    if (key === last) return
    busy = true
    send(rects, viewportWidth)
      .then((answer) => {
        // Only a clear yes turns the CSS glass off; anything else is taken as no glass in this window.
        const on = answer?.supported === true
        if (on) last = key
        else unsupported = true
        if (!stopped) options.onSupported(on)
      })
      .catch((error: unknown) => {
        // Never a transparent card without glass behind it: back to the CSS glass, and try again on the next change.
        console.warn('set_glass', error)
        last = ''
        if (!stopped) options.onSupported(false)
      })
      .finally(() => {
        busy = false
        if (stopped) {
          clear()
          return
        }
        if (again) {
          again = false
          sync()
        }
      })
  }

  function tick(): void {
    sync()
    if (!stopped && following()) {
      frame(tick)
    } else {
      ticking = false
      // Settled: one more report, which drops the shape when the sidebar ended collapsed.
      sync()
    }
  }

  function follow(ms: number): void {
    if (stopped || unsupported) return
    until = Math.max(until, now() + ms)
    if (ticking) return
    ticking = true
    frame(tick)
  }

  /** After stop: the shape goes, once, after any call still in flight, so that call cannot put it back. */
  let cleared = false
  function clear(): void {
    if (cleared || unsupported || last === '') return
    cleared = true
    void send([], window.innerWidth).catch(() => {})
  }

  function stop(): void {
    if (stopped) return
    stopped = true
    // The window keeps running in the menu bar when this page goes; its glass goes with the page.
    if (!busy) clear()
  }

  return { sync, follow, stop }
}
