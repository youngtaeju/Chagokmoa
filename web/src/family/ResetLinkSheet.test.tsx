import { cleanup, screen, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { User } from '@/api/types'
import { initI18n } from '@/i18n'
import { json, mockFetch } from '@/test/fetch'
import { emptyFeedRoutes, settingsFixture, userFixture } from '@/test/fixtures'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const OWNER = userFixture('owner')
const DAD: User = {
  ...userFixture('family'),
  id: '00000000-0000-4000-8000-000000000002',
  username: 'dad',
  display_name: '아빠',
  role: 'guardian',
}
const SUSPENDED: User = {
  ...DAD,
  id: '00000000-0000-4000-8000-000000000004',
  username: 'kid',
  display_name: '정지됨',
  disabled_at: '2026-01-02T00:00:00Z',
}

function renderMembers(extra: Parameters<typeof mockFetch>[0] = {}) {
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => json(200, { user: OWNER }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/users': () => json(200, { users: [OWNER, DAD, SUSPENDED] }),
    'GET /api/invites': () => json(200, { invites: [] }),
    ...extra,
  })
  return renderApp('/family')
}

describe('ResetLinkSheet', () => {
  it('member_sheet_reset_button_hidden_for_suspended', async () => {
    const { user } = renderMembers()
    await user.click(await screen.findByRole('button', { name: /아빠/ }))
    let sheet = await screen.findByRole('dialog')
    expect(
      within(sheet).getByRole('button', { name: '비밀번호 재설정 링크' }),
    ).toBeInTheDocument()
    await user.keyboard('{Escape}')
    await user.click(await screen.findByRole('button', { name: /정지됨/ }))
    sheet = await screen.findByRole('dialog')
    expect(
      within(sheet).queryByRole('button', { name: '비밀번호 재설정 링크' }),
    ).toBeNull()
  })

  it('creates_link_only_on_press', async () => {
    let calls = 0
    const { user } = renderMembers({
      [`POST /api/users/${DAD.id}/password-reset`]: () => {
        calls += 1
        return json(201, {
          url: 'http://x/reset/r',
          expires_at: '2026-01-02T00:00:00Z',
        })
      },
    })
    await user.click(await screen.findByRole('button', { name: /아빠/ }))
    const sheet = await screen.findByRole('dialog')
    await user.click(
      within(sheet).getByRole('button', { name: '비밀번호 재설정 링크' }),
    )
    const reset = await screen.findByRole('dialog', { name: /재설정/ })
    expect(calls).toBe(0)
    expect(within(reset).queryByDisplayValue('http://x/reset/r')).toBeNull()
    await user.click(within(reset).getByRole('button', { name: '링크 만들기' }))
    expect(await screen.findByDisplayValue('http://x/reset/r')).toHaveAttribute(
      'readonly',
    )
    expect(calls).toBe(1)
  })
})
