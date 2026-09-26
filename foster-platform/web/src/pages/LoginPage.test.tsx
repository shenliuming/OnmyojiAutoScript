import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AppRouter } from '../router'

const base = {
  sessionNo: 'session-1', status: 'CREATED', qrExpiresAt: null,
  characterName: null, serverName: null, identityVerified: false,
  identityVerifyReason: null, expiresAt: '2026-09-26T12:00:00Z',
}

function json(data: unknown, status = 200) {
  return new Response(JSON.stringify(data), { status, headers: { 'content-type': 'application/json' } })
}

function mount(hash = '#control=control-token') {
  render(<MemoryRouter initialEntries={[`/login/public-token${hash}`]}><AppRouter /></MemoryRouter>)
}

beforeEach(() => vi.stubGlobal('EventSource', undefined))
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers() })

describe('customer login page', () => {
  it('shows only the cropped QR endpoint and selects Android with the control token', async () => {
    const fetcher = vi.fn(async (path: string) => path.endsWith('/meta')
      ? json({ ...base, status: 'QR_READY', qrExpiresAt: '2026-09-26T12:01:00Z' })
      : json({ status: 'PLATFORM_SELECTED' }))
    vi.stubGlobal('fetch', fetcher)
    mount()
    expect(await screen.findByRole('img', { name: '登录二维码' })).toHaveAttribute(
      'src', '/public/login/public-token/qr?v=2026-09-26T12%3A01%3A00Z',
    )
    fireEvent.click(screen.getByRole('button', { name: '安卓区' }))
    await waitFor(() => expect(fetcher).toHaveBeenCalledWith(
      '/public/login/control-token/platform', expect.objectContaining({ method: 'POST', body: '{"platform":"android"}' }),
    ))
  })

  it('shows identity form after scan and only enables confirmation after backend verification', async () => {
    let state = { ...base, status: 'DETECTING_LOGIN', characterName: null as string | null, serverName: null as string | null }
    const fetcher = vi.fn(async (path: string) => {
      if (path.endsWith('/meta')) return json(state)
      if (path.endsWith('/identity')) return json({ verified: false, waitingForDetection: true, message: '等待 Agent 截图/OCR 校验' })
      return json({ status: 'SUCCESS' })
    })
    vi.stubGlobal('fetch', fetcher)
    mount()
    expect(await screen.findByText('填写账号识别信息')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '确认这是我的账号' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText('区服（可搜索）'), { target: { value: '春之樱' } })
    fireEvent.change(screen.getByLabelText('角色名'), { target: { value: '测试角色' } })
    fireEvent.change(screen.getByLabelText('游戏 UID'), { target: { value: '10001' } })
    fireEvent.click(screen.getByRole('button', { name: '提交，开始校验' }))
    await waitFor(() => expect(fetcher).toHaveBeenCalledWith(
      '/public/login/control-token/identity', expect.objectContaining({ method: 'POST', body: '{"serverName":"春之樱","characterName":"测试角色","gameUid":"10001"}' }),
    ))
    expect(screen.getByRole('button', { name: '确认这是我的账号' })).toBeDisabled()
    state = { ...state, status: 'VERIFYING_ACCOUNT', identityVerified: true, characterName: '测试角色', serverName: '春之樱' }
    fireEvent.click(screen.getByRole('button', { name: '刷新状态' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '确认这是我的账号' })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: '确认这是我的账号' }))
    await waitFor(() => expect(fetcher).toHaveBeenCalledWith(
      '/public/login/control-token/confirm', expect.objectContaining({ method: 'POST', body: '{"confirmed":true}' }),
    ))
  })

  it('shows expiry and network errors without pretending login succeeded', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', { status: 410 })))
    mount()
    expect(await screen.findByText('登录链接已过期')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '确认这是我的账号' })).toBeDisabled()
  })
})
