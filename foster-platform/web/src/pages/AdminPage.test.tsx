import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AdminPage } from './AdminPage'

const host = {
  id: 7, hostCode: 'desk-1', hostname: 'My PC', status: 'ONLINE',
  agentVersion: '0.1', lastHeartbeatAt: null, totalEmulators: 2,
  onlineEmulators: 1, configuredCapacity: 4, boundAccounts: 1,
}
const emulator = {
  id: 8, hostId: 7, emulatorCode: 'device-1', driverType: 'ldplayer',
  maxAccountCount: 4, status: 'ONLINE', adbSerial: 'emulator-5554',
  currentJobId: null, lastHeartbeatAt: null, boundAccounts: 1,
}

function json(data: unknown) {
  return new Response(JSON.stringify(data), { status: 200, headers: { 'Content-Type': 'application/json' } })
}

beforeEach(() => {
  sessionStorage.clear()
  window.history.replaceState({}, '', '/admin')
})
afterEach(() => vi.unstubAllGlobals())

describe('admin dashboard', () => {
  it('prompts for launcher credentials and blocks onboarding when token is absent', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', { status: 200 })))
    render(<AdminPage />)
    expect(await screen.findByText('管理凭据缺失，请从本机一键启动器打开控制台')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '分配模拟器并创建扫码登录' })).toBeDisabled()
  })

  it('shows live host and emulator data from authenticated same-origin APIs', async () => {
    sessionStorage.setItem('foster.adminToken', 'secret')
    const fetchSpy = vi.fn((path: string, _init?: RequestInit) => Promise.resolve(
      path === '/readyz' ? new Response('', { status: 200 }) :
      path === '/admin/hosts' ? json([host]) :
      path === '/admin/hosts/7/emulators' ? json([emulator]) :
      new Response('', { status: 404 }),
    ))
    vi.stubGlobal('fetch', fetchSpy)
    render(<AdminPage />)

    expect(await screen.findByText('device-1')).toBeInTheDocument()
    expect(screen.getByText('emulator-5554')).toBeInTheDocument()
    expect(screen.getByText(/My PC · 1\/2 台在线/)).toBeInTheDocument()
    const calls = fetchSpy.mock.calls as unknown as [string, RequestInit][]
    const adminCall = calls.find(([path]) => path === '/admin/hosts')
    expect(new Headers(adminCall?.[1]?.headers).get('Authorization')).toBe('Bearer secret')
  })

  it('creates customer links, shows both destinations and copies login URL', async () => {
    sessionStorage.setItem('foster.adminToken', 'secret')
    const fetchSpy = vi.fn((path: string, _init?: RequestInit) => Promise.resolve(
      path === '/readyz' ? new Response('', { status: 200 }) :
      path === '/admin/hosts' ? json([host]) :
      path === '/admin/hosts/7/emulators' ? json([emulator]) :
      path === '/admin/onboard' ? json({
        subscriptionNo: 'sub-1', loginUrl: '/login/login-token#control=ctl',
        serviceUrl: '/service/service-token#control=sctl', loginDispatchStatus: 'DISPATCHED',
      }) : new Response('', { status: 404 }),
    ))
    const copy = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: copy } })
    vi.stubGlobal('fetch', fetchSpy)
    render(<AdminPage />)
    await screen.findByText('device-1')

    fireEvent.change(screen.getByLabelText('客户编号'), { target: { value: '10006' } })
    fireEvent.click(screen.getByRole('button', { name: '分配模拟器并创建扫码登录' }))

    const login = await screen.findByRole('link', { name: '打开客户扫码页面（外网）' })
    expect(login).toHaveAttribute('href', 'http://j55d6643.natappfree.cc/login/login-token#control=ctl')
    expect(screen.getByRole('link', { name: '打开服务控制页面（外网）' }))
      .toHaveAttribute('href', 'http://j55d6643.natappfree.cc/service/service-token#control=sctl')
    const request = fetchSpy.mock.calls.find(([path]) => path === '/admin/onboard')
    expect(JSON.parse(String(request?.[1]?.body))).toEqual({
      customerId: 10006, planCode: 'BASIC_AUTO_FOSTER', serviceDays: 30, loginTtlMinutes: 30,
    })
    fireEvent.click(screen.getByRole('button', { name: '复制客户扫码链接' }))
    await waitFor(() => expect(copy).toHaveBeenCalledWith('http://j55d6643.natappfree.cc/login/login-token#control=ctl'))
  })
})
