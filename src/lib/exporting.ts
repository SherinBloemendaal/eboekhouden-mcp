import { reactive } from 'vue'
import { revealExport } from './server'

/** An Exporteer button's state: the file it wrote or why it could not, and showing that file in Finder. */
export interface ExportState {
  exporting: boolean
  /** The absolute path of the file the last export wrote. */
  path: string | null
  /** Why the last export failed. */
  error: string | null
  revealing: boolean
  /** Why showing the file failed, if it did. */
  revealError: string | null
  run(): Promise<void>
  reveal(): Promise<void>
  close(): void
}

/** Shared by every page with an Exporteer button; ExportNotice.vue shows the outcome. `write` gives the file's path. */
export function useExport(write: () => Promise<string>): ExportState {
  const state = reactive<ExportState>({
    exporting: false,
    path: null,
    error: null,
    revealing: false,
    revealError: null,
    async run() {
      if (state.exporting) return
      state.exporting = true
      state.revealError = null
      try {
        state.path = await write()
        state.error = null
      } catch (error) {
        state.path = null
        state.error = String(error)
      } finally {
        state.exporting = false
      }
    },
    async reveal() {
      if (!state.path || state.revealing) return
      state.revealing = true
      state.revealError = null
      try {
        await revealExport(state.path)
      } catch (error) {
        state.revealError = String(error)
      } finally {
        state.revealing = false
      }
    },
    close() {
      state.path = null
      state.error = null
      state.revealError = null
    },
  })
  return state
}
