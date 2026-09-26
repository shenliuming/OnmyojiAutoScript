import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AppRouter } from '../router'

const status = {
  subscriptionNo: 'sub-1', serviceStatus: 'ACTIVE', characterName: '角色', serverName: '春之樱',
  loginStatus: 'SUCCESS', verifyStatus: 'VERIFIED', planName: '基础', resourceMode: 'AUTO',
  resourceType: null, dailyTargetRuns: 2, todaySuccessCount: 1, lastSuccessAt: null,
  nextRunAt: null, manualPauseUntil: null, effectiveBlockedUntil: null, blockReason: null,
  serviceEndAt: '2026-10-26T12:00:00Z', reloginRequired: false, quietPeriods: [], recentJobs: [],
}

afterEach(() => vi.unstubAllGlobals())

describe('customer service page', () => {
  it('shows read-only status and disables controls without a control token', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify(status), { status: 200 })))
    render(<MemoryRouter initialEntries={['/service/public-token']}><AppRouter /></MemoryRouter>)
    expect(await screen.findByText('角色 · 春之樱')).toBeInTheDocument()
    expect(screen.getByText('只读链接')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '暂停 1 小时' })).toBeDisabled()
  })

  it('sends pause and quiet-period commands with the control token', async () => {
    const fetcher = vi.fn(async (path: string) => new Response(
      path === '/r/public-token' ? JSON.stringify(status) : '{}', { status: 200 },
    ))
    vi.stubGlobal('fetch', fetcher)
    render(<MemoryRouter initialEntries={['/service/public-token#control=ctl']}><AppRouter /></MemoryRouter>)
    await screen.findByText('角色 · 春之樱')
    fireEvent.click(screen.getByRole('button', { name: '暂停 1 小时' }))
    await waitFor(() => expect(fetcher).toHaveBeenCalledWith('/r/ctl/pause', expect.objectContaining({
      method: 'POST', body: '{"preset":"1H"}',
    })))
    fireEvent.change(screen.getByLabelText('不上号时间段'), { target: { value: '[{"weekdayMask":1,"startTime":"22:00","endTime":"23:00"}]' } })
    fireEvent.click(screen.getByRole('button', { name: '保存时间段' }))
    await waitFor(() => expect(fetcher).toHaveBeenCalledWith('/r/ctl/quiet-periods', expect.objectContaining({
      method: 'PUT', body: '{"quietPeriods":[{"weekdayMask":1,"startTime":"22:00","endTime":"23:00","beforeBufferMinutes":0,"afterBufferMinutes":0}]}',
    })))
  })
})
