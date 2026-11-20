import { screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import { apiError, emptyFeedRoutes, settingsFixture } from '@/test/fixtures'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
})

function routes(
  mailEnabled: boolean,
  forgot?: (req: Request) => Response | Promise<Response>,
) {
  return {
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => apiError(401, 'auth.required'),
    'GET /api/auth/status': () =>
      json(200, { signup_open: false, mail_enabled: mailEnabled }),
    'GET /api/settings': () => json(200, settingsFixture),
    ...(forgot ? { 'POST /api/auth/forgot-password': forgot } : {}),
  }
}

async function openSheet(
  mailEnabled: boolean,
  forgot?: Parameters<typeof routes>[1],
) {
  mockFetch(routes(mailEnabled, forgot))
  const { user } = renderApp('/login')
  await user.click(
    await screen.findByRole('button', { name: ko.auth.login.forgot }),
  )
  return { user, dialog: await screen.findByRole('dialog') }
}

describe('ForgotPasswordSheet', () => {
  it('forgot_sheet_without_mail', async () => {
    const { dialog } = await openSheet(false)
    expect(dialog).toHaveTextContent(ko.auth.login.forgotAskAdmin)
    expect(screen.queryByLabelText(ko.auth.login.forgotLogin)).toBeNull()
    expect(
      screen.queryByRole('button', { name: ko.auth.login.forgotSend }),
    ).toBeNull()
  })

  it('forgot_sheet_with_mail', async () => {
    let body: unknown
    const { user, dialog } = await openSheet(true, async (req) => {
      body = await req.json()
      return json(204, null)
    })
    await user.type(
      await screen.findByLabelText(ko.auth.login.forgotLogin),
      'mom',
    )
    await user.click(
      screen.getByRole('button', { name: ko.auth.login.forgotSend }),
    )
    expect(await screen.findByText(ko.auth.login.forgotDone)).toBeVisible()
    expect(body).toEqual({ login: 'mom' })
    expect(screen.queryByLabelText(ko.auth.login.forgotLogin)).toBeNull()
    expect(dialog).not.toHaveTextContent(ko.auth.login.forgotAskAdmin)
  })

  it('forgot_sheet_send_error', async () => {
    const { user } = await openSheet(true, () => {
      throw new TypeError('Failed to fetch')
    })
    await user.type(
      await screen.findByLabelText(ko.auth.login.forgotLogin),
      'mom',
    )
    await user.click(
      screen.getByRole('button', { name: ko.auth.login.forgotSend }),
    )
    expect(await screen.findByRole('alert')).toBeVisible()
    expect(screen.getByLabelText(ko.auth.login.forgotLogin)).toHaveValue('mom')
    expect(screen.queryByText(ko.auth.login.forgotDone)).toBeNull()
  })

  it('forgot_sheet_resets_on_reopen', async () => {
    const { user } = await openSheet(true, () => json(204, null))
    await user.type(
      await screen.findByLabelText(ko.auth.login.forgotLogin),
      'mom',
    )
    await user.click(
      screen.getByRole('button', { name: ko.auth.login.forgotSend }),
    )
    expect(await screen.findByText(ko.auth.login.forgotDone)).toBeVisible()

    await user.keyboard('{Escape}')
    expect(screen.queryByRole('dialog')).toBeNull()
    await user.click(screen.getByRole('button', { name: ko.auth.login.forgot }))
    expect(
      await screen.findByLabelText(ko.auth.login.forgotLogin),
    ).toBeVisible()
    expect(screen.queryByText(ko.auth.login.forgotDone)).toBeNull()
  })
})
