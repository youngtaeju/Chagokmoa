import { screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import type { components } from '@/api/schema'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import { emptyFeedRoutes, settingsFixture, userFixture } from '@/test/fixtures'
import { renderApp } from '@/test/router'

type Method = components['schemas']['LoginMethod']

beforeEach(() => {
  initI18n('ko')
})

function render(
  method: () => Method,
  extra = {},
  path = '/settings/login-method',
) {
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => json(200, { user: userFixture('family') }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/auth/status': () =>
      json(200, { signup_open: false, oidc: { label: 'Authentik' } }),
    'GET /api/auth/login-method': () => json(200, method()),
    ...extra,
  })
  return renderApp(path)
}

describe('login method page', () => {
  it('login_method_when_oidc_off', async () => {
    render(() => ({ has_password: true, oidc: 'none' }), {
      'GET /api/auth/status': () =>
        json(200, { signup_open: false, oidc: null }),
    })
    expect(await screen.findByText(ko.errors.oidc.disabled)).toBeInTheDocument()
    expect(screen.queryByText(ko.common.loading)).toBeNull()
  })

  it('link_states', async () => {
    render(() => ({ has_password: true, oidc: 'none' }))
    expect(
      await screen.findByRole('heading', {
        name: `${ko.me.account} ${ko.me.loginMethod}`,
      }),
    ).toBeInTheDocument()
    expect(await screen.findByText(ko.loginMethod.none)).toBeInTheDocument()
    expect(
      screen.getByRole('link', { name: 'Authentik 계정 연결' }),
    ).toHaveAttribute('href', '/api/auth/oidc/start?purpose=link')
  })

  it('link_state_linked', async () => {
    render(() => ({ has_password: true, oidc: 'linked' }))
    expect(await screen.findByText(ko.loginMethod.linked)).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: ko.loginMethod.unlink }),
    ).toBeEnabled()
  })

  it('link_state_stale', async () => {
    render(() => ({ has_password: true, oidc: 'stale' }))
    expect(await screen.findByText(ko.loginMethod.stale)).toBeInTheDocument()
    expect(
      screen.getByRole('link', { name: 'Authentik 계정 연결' }),
    ).toHaveAttribute('href', '/api/auth/oidc/start?purpose=link')
  })

  it('unlink_disabled_without_password', async () => {
    render(() => ({ has_password: false, oidc: 'linked' }))
    const button = await screen.findByRole('button', {
      name: ko.loginMethod.unlink,
    })
    expect(button).toBeDisabled()
    expect(screen.getByText(ko.errors.oidc.needPassword)).toBeInTheDocument()
    expect(screen.getByRole('link', { name: ko.me.password })).toHaveAttribute(
      'href',
      '/settings/password',
    )
  })

  it('unlink_calls_delete', async () => {
    let state: Method = { has_password: true, oidc: 'linked' }
    let deleted = 0
    const { user } = render(() => state, {
      'DELETE /api/auth/oidc/link': () => {
        deleted += 1
        state = { has_password: true, oidc: 'none' }
        return json(204, null)
      },
    })
    await user.click(
      await screen.findByRole('button', { name: ko.loginMethod.unlink }),
    )
    await waitFor(() => {
      expect(deleted).toBe(1)
    })
    expect(await screen.findByText(ko.loginMethod.none)).toBeInTheDocument()
  })

  it('shows_oidc_error', async () => {
    const { router } = render(
      () => ({ has_password: true, oidc: 'none' }),
      {},
      '/settings/login-method?oidc_error=oidc.alreadyLinked',
    )
    expect(
      await screen.findByText(ko.errors.oidc.alreadyLinked),
    ).toBeInTheDocument()
    await waitFor(() => {
      expect(router.state.location.search).not.toHaveProperty('oidc_error')
    })
    expect(screen.getByText(ko.errors.oidc.alreadyLinked)).toBeInTheDocument()
  })
})
