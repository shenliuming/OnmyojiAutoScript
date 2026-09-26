import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { apiFetch } from './api'
import { readAdminToken } from './adminAuth'

beforeEach(() => {
  sessionStorage.clear()
  window.history.replaceState({}, '', '/admin')
})

afterEach(() => vi.unstubAllGlobals())

describe('admin token', () => {
  it('moves token from fragment into tab storage and retains other hash values', () => {
    window.history.replaceState({}, '', '/admin?tab=hosts#panel=devices&token=top-secret&mode=compact')
    expect(readAdminToken()).toBe('top-secret')
    expect(window.location.pathname + window.location.search + window.location.hash)
      .toBe('/admin?tab=hosts#panel=devices&mode=compact')
    expect(sessionStorage.getItem('foster.adminToken')).toBe('top-secret')
    expect(readAdminToken()).toBe('top-secret')
  })

  it('removes an empty token fragment without storing a credential', () => {
    window.history.replaceState({}, '', '/admin#token=')
    expect(readAdminToken()).toBeNull()
    expect(window.location.hash).toBe('')
  })

  it('never reads the admin token from the query string', () => {
    window.history.replaceState({}, '', '/admin?token=query-secret')
    expect(readAdminToken()).toBeNull()
  })
})

describe('admin API requests', () => {
  it('sends stored credentials only as a bearer header on a relative same-origin path', async () => {
    sessionStorage.setItem('foster.adminToken', 'top-secret')
    const fetchSpy = vi.fn().mockResolvedValue(new Response('{}', { status: 200 }))
    vi.stubGlobal('fetch', fetchSpy)

    await apiFetch('/admin/hosts?active=1', { headers: { Accept: 'application/json' } })

    expect(fetchSpy).toHaveBeenCalledOnce()
    const [url, init] = fetchSpy.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/admin/hosts?active=1')
    expect(url).not.toContain('top-secret')
    expect(new Headers(init.headers).get('Authorization')).toBe('Bearer top-secret')
    expect(new Headers(init.headers).get('Accept')).toBe('application/json')
    expect(init.referrerPolicy).toBe('no-referrer')
  })

  it('rejects an admin request without a token before fetching', async () => {
    const fetchSpy = vi.fn()
    vi.stubGlobal('fetch', fetchSpy)
    await expect(apiFetch('/admin/hosts')).rejects.toThrow('请从本机一键启动器打开控制台')
    expect(fetchSpy).not.toHaveBeenCalled()
  })

  it('rejects absolute URLs so credentials cannot leave this origin', async () => {
    sessionStorage.setItem('foster.adminToken', 'top-secret')
    await expect(apiFetch('https://example.com/admin/hosts')).rejects.toThrow('必须使用本站相对 API 地址')
  })
})
