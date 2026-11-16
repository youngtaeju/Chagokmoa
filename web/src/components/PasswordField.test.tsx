import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it } from 'vitest'
import { initI18n } from '@/i18n'
import { PasswordField } from './PasswordField'

beforeEach(() => {
  initI18n('ko')
})

describe('PasswordField', () => {
  it('toggle_switches_type_and_pressed', async () => {
    const user = userEvent.setup()
    render(<PasswordField label="비밀번호" name="pw" />)
    const input = screen.getByLabelText('비밀번호')
    expect(input).toHaveAttribute('type', 'password')
    const btn = screen.getByRole('button', { name: '비밀번호 보기' })
    expect(btn).toHaveAttribute('aria-pressed', 'false')
    await user.click(btn)
    expect(input).toHaveAttribute('type', 'text')
    await user.click(screen.getByRole('button', { name: '비밀번호 숨기기' }))
    expect(input).toHaveAttribute('type', 'password')
  })
})
