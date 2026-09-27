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

describe('customer shortest login path', () => {
  it('submits platform character and role id before emulator dispatch', async () => {
    const fetcher = vi.fn(async (path: string) => {
      if (path.endsWith('/meta')) return json(base)
      if (path.endsWith('/start')) return json({ status: 'WAITING_EMULATOR' })
      return json({})
    })
    vi.stubGlobal('fetch', fetcher)
    mount()

    expect(await screen.findByRole('heading', { name: '开始登录' })).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('游戏平台'), { target: { value: 'ios' } })
    fireEvent.change(screen.getByLabelText('角色名'), { target: { value: '测试角色' } })
    fireEvent.change(screen.getByLabelText('角色 ID'), { target: { value: '10001' } })
    fireEvent.click(screen.getByRole('button', { name: '开始登录' }))

    await waitFor(() => expect(fetcher).toHaveBeenCalledWith(
      '/public/login/control-token/start',
      expect.objectContaining({
        method: 'POST',
        body: '{"platform":"ios","characterName":"测试角色","gameUid":"10001"}',
      }),
    ))
  })

  it('shows QR without asking for platform or server again', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({
      ...base,
      status: 'QR_READY',
      qrExpiresAt: '2026-09-26T12:01:00Z',
    })))
    mount()

    expect(await screen.findByRole('img', { name: '登录二维码' })).toHaveAttribute(
      'src', '/public/login/public-token/qr?v=2026-09-26T12%3A01%3A00Z',
    )
    expect(screen.queryByText('安卓区')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('区服（可搜索）')).not.toBeInTheDocument()
    expect(screen.getByText('扫码后无需继续操作，系统会自动选择你填写的平台和角色。')).toBeInTheDocument()
  })

  it('only enables final confirmation after detected character and server are verified', async () => {
    let state = {
      ...base,
      status: 'VERIFYING_ACCOUNT',
      identityVerified: true,
      characterName: '测试角色',
      serverName: '春之樱',
    }
    const fetcher = vi.fn(async (path: string) => {
      if (path.endsWith('/meta')) return json(state)
      if (path.endsWith('/confirm')) {
        state = { ...state, status: 'SUCCESS' }
        return json({ status: 'SUCCESS' })
      }
      return json({})
    })
    vi.stubGlobal('fetch', fetcher)
    mount()

    const confirm = await screen.findByRole('button', { name: '确认这是我的角色' })
    expect(confirm).toBeEnabled()
    expect(screen.getByText('已识别：测试角色 · 春之樱。请确认这是你的角色。')).toBeInTheDocument()

    fireEvent.click(confirm)
    await waitFor(() => expect(fetcher).toHaveBeenCalledWith(
      '/public/login/control-token/confirm',
      expect.objectContaining({ method: 'POST', body: '{"confirmed":true}' }),
    ))
  })

  it('shows expiry without pretending login succeeded', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', { status: 410 })))
    mount()
    expect(await screen.findByText('登录链接已过期')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '确认这是我的角色' })).toBeDisabled()
  })
})
