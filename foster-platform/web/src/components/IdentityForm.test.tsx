import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { IdentityForm } from './IdentityForm'

describe('quick login target entry', () => {
  it('requires character and role id and submits the selected platform', () => {
    const submit = vi.fn()
    render(<IdentityForm busy={false} onSubmit={submit} />)

    expect(screen.getByLabelText('游戏平台')).toHaveValue('android')
    fireEvent.change(screen.getByLabelText('游戏平台'), { target: { value: 'ios' } })
    fireEvent.change(screen.getByLabelText('角色名'), { target: { value: '测试角色' } })
    fireEvent.click(screen.getByRole('button', { name: '开始登录' }))

    expect(screen.getByText('请选择平台，并填写角色名和角色 ID')).toBeInTheDocument()
    expect(submit).not.toHaveBeenCalled()

    fireEvent.change(screen.getByLabelText('角色 ID'), { target: { value: '10001' } })
    fireEvent.click(screen.getByRole('button', { name: '开始登录' }))

    expect(submit).toHaveBeenCalledWith({
      platform: 'ios',
      characterName: '测试角色',
      gameUid: '10001',
    })
  })
})
