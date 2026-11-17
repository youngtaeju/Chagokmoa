import { screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { meQueryKey } from '@/auth/session'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import {
  apiError,
  emptyFeedRoutes,
  settingsFixture,
  userFixture,
} from '@/test/fixtures'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
})

const raw = 'Rz3_x-9QzT'
const preview = {
  display_name: '아빠',
  username: 'dad',
  expires_at: '2026-11-10T00:00:00Z',
}

function routes(me: boolean, extra: Parameters<typeof mockFetch>[0] = {}) {
  return {
    ...emptyFeedRoutes,
    'GET /api/auth/me': () =>
      me
        ? json(200, { user: userFixture('owner') })
        : apiError(401, 'auth.required'),
    'GET /api/auth/status': () => json(200, { signup_open: false }),
    'GET /api/settings': () => json(200, settingsFixture),
    ...extra,
  }
}

describe('PasswordResetPage', () => {
  it('shows_name_and_username', async () => {
    mockFetch(
      routes(false, {
        [`GET /api/password-resets/${raw}/preview`]: () => json(200, preview),
      }),
    )
    renderApp(`/reset/${raw}`)
    expect(
      await screen.findByRole('heading', { name: '아빠님의 새 비밀번호' }),
    ).toBeInTheDocument()
    expect(screen.getByText('아이디: dad')).toBeInTheDocument()
  })

  it('mismatch_blocks_request', async () => {
    let accepted = 0
    mockFetch(
      routes(false, {
        [`GET /api/password-resets/${raw}/preview`]: () => json(200, preview),
        [`POST /api/password-resets/${raw}/accept`]: () => {
          accepted += 1
          return json(200, { user: userFixture('guardian') })
        },
      }),
    )
    const { user } = renderApp(`/reset/${raw}`)
    await user.type(
      await screen.findByLabelText(ko.auth.password),
      'password123',
    )
    await user.type(
      screen.getByLabelText(ko.auth.passwordConfirm),
      'different1',
    )
    await user.click(screen.getByRole('button', { name: ko.auth.reset.submit }))
    expect(accepted).toBe(0)
    expect(screen.getByText(ko.auth.passwordMismatch)).toBeInTheDocument()
  })

  it('accept_goes_home', async () => {
    let body: unknown
    mockFetch(
      routes(false, {
        [`GET /api/password-resets/${raw}/preview`]: () => json(200, preview),
        [`POST /api/password-resets/${raw}/accept`]: async (req) => {
          body = await req.json()
          return json(200, { user: userFixture('guardian') })
        },
      }),
    )
    const { router, user, queryClient } = renderApp(`/reset/${raw}`)
    await user.type(
      await screen.findByLabelText(ko.auth.password),
      'password123',
    )
    await user.type(
      screen.getByLabelText(ko.auth.passwordConfirm),
      'password123',
    )
    await user.click(screen.getByRole('button', { name: ko.auth.reset.submit }))
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/')
    })
    expect(body).toEqual({ password: 'password123' })
    expect(queryClient.getQueryData(meQueryKey)).toEqual(
      userFixture('guardian'),
    )
  })

  it.each([
    ['expired', 410],
    ['used', 410],
    ['revoked', 410],
    ['notFound', 404],
  ] as const)('invalid_link %s', async (key, status) => {
    mockFetch(
      routes(false, {
        [`GET /api/password-resets/${raw}/preview`]: () =>
          apiError(status, `passwordReset.${key}`),
      }),
    )
    renderApp(`/reset/${raw}`)
    expect(
      await screen.findByRole('heading', { name: ko.auth.reset.error[key] }),
    ).toBeInTheDocument()
    expect(
      screen.getByText('관리자에게 새 링크를 요청해 주세요.'),
    ).toBeInTheDocument()
    expect(
      screen.getByRole('link', { name: ko.auth.toLogin }),
    ).toBeInTheDocument()
  })

  it('accept_410_switches_to_invalid_view', async () => {
    mockFetch(
      routes(false, {
        [`GET /api/password-resets/${raw}/preview`]: () => json(200, preview),
        [`POST /api/password-resets/${raw}/accept`]: () =>
          apiError(410, 'passwordReset.used'),
      }),
    )
    const { user } = renderApp(`/reset/${raw}`)
    await user.type(
      await screen.findByLabelText(ko.auth.password),
      'password123',
    )
    await user.type(
      screen.getByLabelText(ko.auth.passwordConfirm),
      'password123',
    )
    await user.click(screen.getByRole('button', { name: ko.auth.reset.submit }))
    expect(
      await screen.findByRole('heading', { name: ko.auth.reset.error.used }),
    ).toBeInTheDocument()
  })

  it('reset_page_opens_when_signed_in', async () => {
    mockFetch(
      routes(true, {
        [`GET /api/password-resets/${raw}/preview`]: () => json(200, preview),
      }),
    )
    const { router } = renderApp(`/reset/${raw}`)
    expect(
      await screen.findByRole('heading', { name: '아빠님의 새 비밀번호' }),
    ).toBeInTheDocument()
    expect(router.state.location.pathname).toBe(`/reset/${raw}`)
  })

  it('accept_clears_previous_user_data', async () => {
    mockFetch(
      routes(true, {
        [`GET /api/password-resets/${raw}/preview`]: () => json(200, preview),
        [`POST /api/password-resets/${raw}/accept`]: () =>
          json(200, { user: userFixture('guardian') }),
      }),
    )
    const { router, user, queryClient } = renderApp(`/reset/${raw}`)
    queryClient.setQueryData(['notifications'], { items: ['이전 사용자'] })
    await user.type(
      await screen.findByLabelText(ko.auth.password),
      'password123',
    )
    await user.type(
      screen.getByLabelText(ko.auth.passwordConfirm),
      'password123',
    )
    await user.click(screen.getByRole('button', { name: ko.auth.reset.submit }))
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/')
    })
    expect(queryClient.getQueryData(['notifications'])).toBeUndefined()
    expect(queryClient.getQueryData(meQueryKey)).toEqual(
      userFixture('guardian'),
    )
    expect(
      screen.queryByRole('heading', { name: ko.auth.reset.error.used }),
    ).not.toBeInTheDocument()
  })
})
