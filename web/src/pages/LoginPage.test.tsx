import { screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { initI18n } from '@/i18n'
import { json, mockFetch } from '@/test/fetch'
import { apiError, emptyFeedRoutes, settingsFixture } from '@/test/fixtures'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
})

describe('LoginPage tagline', () => {
  it('emphasis_has_underline_svg_in_ko_and_en', async () => {
    mockFetch({
      ...emptyFeedRoutes,
      'GET /api/auth/me': () => apiError(401, 'auth.required'),
      'GET /api/auth/status': () => json(200, { signup_open: false }),
      'GET /api/settings': () => json(200, settingsFixture),
    })
    const { user } = renderApp('/login')
    expect(
      (await screen.findByText('차곡차곡')).querySelector('svg'),
    ).not.toBeNull()
    await user.click(screen.getByRole('button', { name: 'English' }))
    expect(
      (await screen.findByText('one by one')).querySelector('svg'),
    ).not.toBeNull()
  })
})
