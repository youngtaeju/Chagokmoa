import { cleanup, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { components } from '@/api/schema'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import { emptyFeedRoutes, settingsFixture, userFixture } from '@/test/fixtures'
import { renderApp } from '@/test/router'

type OidcResponse = components['schemas']['OidcSettingsResponse']

beforeEach(() => {
  initI18n('ko')
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const CALLBACK = 'https://photos.example.com/api/auth/oidc/callback'

const SAVED: OidcResponse = {
  config: {
    issuer_url: 'https://auth.example.com/',
    client_id: 'chagokmoa',
    secret_set: true,
    button_label: 'Authentik',
  },
  callback_url: CALLBACK,
  linked: 3,
  passwordless: 1,
}

const EMPTY: OidcResponse = {
  config: null,
  callback_url: CALLBACK,
  linked: 0,
  passwordless: 0,
}

function renderOidc(
  res: OidcResponse,
  extra: Parameters<typeof mockFetch>[0] = {},
) {
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => json(200, { user: userFixture('owner') }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/oidc': () => json(200, res),
    ...extra,
  })
  return renderApp('/settings/login')
}

describe('oidc settings', () => {
  it('shows_callback_url_with_copy', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    const { user } = renderOidc(EMPTY)
    // user-event 의 clipboard 스텁을 setup 뒤에 덮어씀.
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    })
    const input = await screen.findByLabelText(ko.oidc.callbackUrl)
    expect(input).toHaveValue(CALLBACK)
    expect(input).toHaveAttribute('readonly')
    expect(screen.queryByRole('button', { name: ko.members.share })).toBeNull()
    await user.click(screen.getByRole('button', { name: ko.common.copy }))
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(CALLBACK)
    })
  })

  it('copy_does_not_submit', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    let puts = 0
    const { user } = renderOidc(SAVED, {
      'PUT /api/oidc': () => {
        puts += 1
        return json(200, SAVED)
      },
    })
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    })
    await screen.findByLabelText(ko.oidc.callbackUrl)
    await user.click(screen.getByRole('button', { name: ko.common.copy }))
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(CALLBACK)
    })
    expect(puts).toBe(0)
  })

  it('secret_placeholder_when_set', async () => {
    let body: Record<string, unknown> | null = null
    const { user } = renderOidc(SAVED, {
      'PUT /api/oidc': async (req) => {
        body = (await req.json()) as Record<string, unknown>
        return json(200, SAVED)
      },
    })
    const secret = await screen.findByLabelText(ko.oidc.clientSecret)
    expect(secret).toHaveValue('')
    expect(secret).toHaveAttribute('placeholder', ko.oidc.secretSet)
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    await screen.findByText(ko.oidc.saved)
    expect(body).not.toBeNull()
    expect(body).not.toHaveProperty('client_secret')
    expect(body).toMatchObject({
      issuer_url: 'https://auth.example.com/',
      client_id: 'chagokmoa',
      button_label: 'Authentik',
    })
  })

  it('secret_cleared_after_save', async () => {
    const bodies: Record<string, unknown>[] = []
    const { user } = renderOidc(SAVED, {
      'PUT /api/oidc': async (req) => {
        bodies.push((await req.json()) as Record<string, unknown>)
        return json(200, SAVED)
      },
    })
    await user.type(
      await screen.findByLabelText(ko.oidc.clientSecret),
      's3cret',
    )
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    await waitFor(() => {
      expect(bodies).toHaveLength(1)
    })
    expect(bodies[0]).toMatchObject({ client_secret: 's3cret' })
    await waitFor(() => {
      expect(screen.getByLabelText(ko.oidc.clientSecret)).toHaveValue('')
    })
  })

  it('discovery_error_shown', async () => {
    const { user } = renderOidc(EMPTY, {
      'PUT /api/oidc': () =>
        json(400, {
          error: 'oidc.discoveryFailed',
          params: { message: 'connection refused' },
        }),
    })
    await user.type(
      await screen.findByLabelText(ko.oidc.issuerUrl),
      'http://127.0.0.1:9/',
    )
    await user.type(screen.getByLabelText(ko.oidc.clientId), 'cid')
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    expect(
      await screen.findByText(
        'IdP 정보를 가져오지 못했어요: connection refused',
      ),
    ).toBeInTheDocument()
  })

  it('warns_on_issuer_change', async () => {
    const bodies: Record<string, unknown>[] = []
    const { user } = renderOidc(SAVED, {
      'PUT /api/oidc': async (req) => {
        bodies.push((await req.json()) as Record<string, unknown>)
        return json(200, { ...SAVED, linked: 0, passwordless: 0 })
      },
    })
    const issuer = await screen.findByLabelText(ko.oidc.issuerUrl)
    await user.clear(issuer)
    await user.type(issuer, 'https://other.example.com/')
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    const dialog = await screen.findByRole('alertdialog')
    expect(
      within(dialog).getByText(
        '연결된 계정 3개 중 1개는 비밀번호가 없어 로그인할 수 없게 돼요. 재설정 링크로 비밀번호를 만들게 해 주세요.',
      ),
    ).toBeInTheDocument()
    await user.click(
      within(dialog).getByRole('button', { name: ko.common.cancel }),
    )
    expect(bodies).toHaveLength(0)

    await user.click(screen.getByRole('button', { name: ko.common.save }))
    const again = await screen.findByRole('alertdialog')
    await user.click(
      within(again).getByRole('button', {
        name: ko.oidc.changeIssuerAction,
      }),
    )
    await waitFor(() => {
      expect(bodies).toHaveLength(1)
    })
    expect(bodies[0]).toMatchObject({
      issuer_url: 'https://other.example.com/',
    })
  })

  it('no_warning_without_passwordless', async () => {
    let puts = 0
    const res = { ...SAVED, passwordless: 0 }
    const { user } = renderOidc(res, {
      'PUT /api/oidc': () => {
        puts += 1
        return json(200, res)
      },
    })
    const issuer = await screen.findByLabelText(ko.oidc.issuerUrl)
    await user.clear(issuer)
    await user.type(issuer, 'https://other.example.com/')
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    await waitFor(() => {
      expect(puts).toBe(1)
    })
    expect(screen.queryByRole('alertdialog')).toBeNull()
  })

  it('delete_warns_and_clears', async () => {
    let deleted = 0
    const { user } = renderOidc(SAVED, {
      'DELETE /api/oidc': () => {
        deleted += 1
        return json(204, null)
      },
    })
    await user.click(
      await screen.findByRole('button', { name: ko.oidc.delete }),
    )
    const dialog = await screen.findByRole('alertdialog')
    expect(
      within(dialog).getByText(/3개 중 1개는 비밀번호가 없어/),
    ).toBeInTheDocument()
    await user.click(
      within(dialog).getByRole('button', { name: ko.oidc.deleteAction }),
    )
    await waitFor(() => {
      expect(deleted).toBe(1)
    })
    await waitFor(() => {
      expect(screen.getByLabelText(ko.oidc.issuerUrl)).toHaveValue('')
    })
    expect(screen.queryByRole('button', { name: ko.oidc.delete })).toBeNull()
  })

  it('clear_secret_sends_empty', async () => {
    const bodies: Record<string, unknown>[] = []
    const { user } = renderOidc(SAVED, {
      'PUT /api/oidc': async (req) => {
        bodies.push((await req.json()) as Record<string, unknown>)
        return json(200, {
          ...SAVED,
          config: { ...SAVED.config, secret_set: false },
        })
      },
    })
    const secret = await screen.findByLabelText(ko.oidc.clientSecret)
    expect(screen.getByText(ko.oidc.secretHintSet)).toBeInTheDocument()
    await user.click(screen.getByLabelText(ko.oidc.clearSecret))
    expect(secret).toBeDisabled()
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    await waitFor(() => {
      expect(bodies).toHaveLength(1)
    })
    expect(bodies[0]).toMatchObject({ client_secret: '' })
  })

  it('no_clear_checkbox_without_secret', async () => {
    renderOidc({
      ...SAVED,
      config: {
        issuer_url: 'https://auth.example.com/',
        client_id: 'chagokmoa',
        secret_set: false,
        button_label: 'Authentik',
      },
    })
    await screen.findByLabelText(ko.oidc.clientSecret)
    expect(screen.queryByLabelText(ko.oidc.clearSecret)).toBeNull()
    expect(screen.getByText(ko.oidc.secretHint)).toBeInTheDocument()
  })

  it('whitespace_secret_is_omitted', async () => {
    const bodies: Record<string, unknown>[] = []
    const { user } = renderOidc(SAVED, {
      'PUT /api/oidc': async (req) => {
        bodies.push((await req.json()) as Record<string, unknown>)
        return json(200, SAVED)
      },
    })
    await user.type(await screen.findByLabelText(ko.oidc.clientSecret), '   ')
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    await waitFor(() => {
      expect(bodies).toHaveLength(1)
    })
    expect(bodies[0]).not.toHaveProperty('client_secret')
  })

  it('trailing_slash_edit_no_warning', async () => {
    let puts = 0
    const { user } = renderOidc(SAVED, {
      'PUT /api/oidc': () => {
        puts += 1
        return json(200, SAVED)
      },
    })
    const issuer = await screen.findByLabelText(ko.oidc.issuerUrl)
    await user.clear(issuer)
    await user.type(issuer, 'https://auth.example.com')
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    await waitFor(() => {
      expect(puts).toBe(1)
    })
    expect(screen.queryByRole('alertdialog')).toBeNull()
  })
})
