import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { IdentityForm } from './IdentityForm'

describe('identity entry', () => {
  it('keeps the server searchable without inventing options and rejects missing UID', () => {
    const submit = vi.fn()
    render(<IdentityForm busy={false} onSubmit={submit} />)
    const server = screen.getByLabelText('区服（可搜索）')
    expect(server).toHaveAttribute('list', 'serverOptions')
    expect(document.querySelectorAll('#serverOptions option')).toHaveLength(0)
    fireEvent.change(server, { target: { value: '春之樱' } })
    fireEvent.change(screen.getByLabelText('角色名'), { target: { value: '测试角色' } })
    fireEvent.click(screen.getByRole('button', { name: '提交，开始校验' }))
    expect(screen.getByText('请填写区服、角色名和游戏 UID')).toBeInTheDocument()
    expect(submit).not.toHaveBeenCalled()
  })
})
