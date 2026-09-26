import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import type { ProxyOptions } from 'vite'
import { BACKEND_PROXY_PATTERNS } from './src/lib/proxyPaths'

const backendTarget = 'http://127.0.0.1:8080'
const allowedHosts = ['j55d6643.natappfree.cc']
const proxy = Object.fromEntries(BACKEND_PROXY_PATTERNS.map((path) => [
  path,
  { target: backendTarget, changeOrigin: false, ...(path.includes('/agent/ws') ? { ws: true } : {}) } satisfies ProxyOptions,
])) as Record<string, ProxyOptions>

export default defineConfig({
  plugins: [react()],
  server: { host: '127.0.0.1', port: 5173, strictPort: true, allowedHosts, proxy },
  preview: { host: '127.0.0.1', port: 5173, strictPort: true, allowedHosts, proxy },
  test: { environment: 'jsdom', setupFiles: ['./src/testSetup.ts'], globals: true },
})
