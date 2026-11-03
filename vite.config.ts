/// <reference types="vitest/config" />
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

// Tauri expects a fixed port and no cleared screen, so its own output stays visible.
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: { port: 5174, strictPort: true },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: { target: 'safari16', outDir: 'dist', emptyOutDir: true },
  test: { environment: 'jsdom' },
})
