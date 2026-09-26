// @vitest-environment node
import { describe, expect, it } from 'vitest'
import config from '../../vite.config'
import { isBackendProxyPath } from './proxyPaths'

describe('backend proxy paths', () => {
  it.each([
    ['/admin/hosts', true],
    ['/admin/onboard?new=1', true],
    ['/public/login/abc/status', true],
    ['/r/abc', true],
    ['/healthz', true],
    ['/readyz?probe=1', true],
    ['/agent/ws', true],
    ['/agent/ws?token=abc', true],
    ['/admin', false],
    ['/administrator', false],
    ['/login/abc', false],
    ['/service/abc', false],
    ['/agent/ws-extra', false],
    ['/healthz-extra', false],
    ['/assets/main.js', false],
  ])('classifies %s as backend=%s', (path, expected) => {
    expect(isBackendProxyPath(path)).toBe(expected)
  })

  it('applies the same backend route contract to dev and preview', () => {
    const devProxy = config.server?.proxy ?? {}
    const previewProxy = config.preview?.proxy ?? {}
    for (const [path, expected] of [
      ['/admin', false],
      ['/admin/hosts', true],
      ['/public/login/abc/events', true],
      ['/r/abc', true],
      ['/login/abc', false],
      ['/service/abc', false],
      ['/agent/ws', true],
    ] as const) {
      for (const proxy of [devProxy, previewProxy]) {
        const matchingRules = Object.entries(proxy).filter(([rule]) => rule.startsWith('^') ? new RegExp(rule).test(path) : path.startsWith(rule))
        expect(matchingRules.length, path).toBe(expected ? 1 : 0)
        if (expected) {
          const options = matchingRules[0][1]
          if (typeof options === 'string') throw new Error('Expected explicit proxy options')
          expect(options).toMatchObject({ target: 'http://127.0.0.1:8080', changeOrigin: false })
          expect(options.rewrite).toBeUndefined()
          expect(options.selfHandleResponse).toBeUndefined()
          if (path === '/agent/ws') expect(options.ws).toBe(true)
        }
      }
    }
    expect(config.server?.port).toBe(5173)
    expect(config.server?.strictPort).toBe(true)
  })
})
