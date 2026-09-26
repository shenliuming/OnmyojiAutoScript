import { render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AppRouter } from './router'

function at(path: string) {
  render(<MemoryRouter initialEntries={[path]}><AppRouter /></MemoryRouter>)
}

describe('application routes', () => {
  beforeEach(() => {
    sessionStorage.clear()
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', { status: 200 })))
  })
  afterEach(() => vi.unstubAllGlobals())

  it('shows the admin console at /admin', async () => {
    at('/admin')
    expect(screen.getByRole('heading', { name: '阴阳师寄养控制台' })).toBeInTheDocument()
    await screen.findByText('管理凭据缺失，请从本机一键启动器打开控制台')
  })

  it('shows the customer login page at /login/:token', () => {
    at('/login/customer-token')
    expect(screen.getByRole('heading', { name: '游戏账号登录' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '确认这是我的账号' })).toBeDisabled()
  })

  it('shows the customer service page at /service/:token', () => {
    at('/service/customer-token')
    expect(screen.getByRole('heading', { name: '我的寄养服务' })).toBeInTheDocument()
  })

  it('shows not found for unknown paths', () => {
    at('/unknown')
    expect(screen.getByRole('heading', { name: '页面不存在' })).toBeInTheDocument()
  })
})
