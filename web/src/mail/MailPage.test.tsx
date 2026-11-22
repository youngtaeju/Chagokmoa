import { cleanup, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { components } from '@/api/schema'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import { emptyFeedRoutes, settingsFixture, userFixture } from '@/test/fixtures'
import { renderApp } from '@/test/router'
import { presetOf } from './presets'

type MailConfig = components['schemas']['MailConfigView']

beforeEach(() => {
  initI18n('ko')
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const SAVED: MailConfig = {
  host: 'smtp.naver.com',
  port: 587,
  security: 'starttls',
  username: 'me@naver.com',
  password_set: true,
  from_address: 'me@naver.com',
}

function renderMail(
  config: MailConfig | null,
  extra: Parameters<typeof mockFetch>[0] = {},
  email: string | null = null,
) {
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () =>
      json(200, { user: { ...userFixture('owner'), email } }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/mail': () => json(200, { config }),
    ...extra,
  })
  return renderApp('/settings/mail')
}

describe('presetOf', () => {
  it('matches_values', () => {
    expect(presetOf('smtp.gmail.com', 587, 'starttls')).toBe('gmail')
    expect(presetOf('smtp.naver.com', 587, 'starttls')).toBe('naver')
    expect(presetOf('smtp.gmail.com', 465, 'tls')).toBe('custom')
  })
})

describe('mail settings', () => {
  it('preset_fills_fields', async () => {
    const { user } = renderMail(null)
    const host = await screen.findByLabelText(ko.mail.host)
    await user.type(host, 'relay.local')
    await user.clear(screen.getByLabelText(ko.mail.port))
    await user.type(screen.getByLabelText(ko.mail.port), '2525')
    await user.selectOptions(
      screen.getByLabelText(ko.mail.security),
      ko.mail.security_none,
    )
    expect(screen.getByLabelText(ko.mail.host)).toHaveValue('relay.local')
    expect(screen.getByLabelText(ko.mail.port)).toHaveValue(2525)
    await user.selectOptions(
      screen.getByLabelText(ko.mail.service),
      ko.mail.presetGmail,
    )
    expect(screen.getByLabelText(ko.mail.host)).toHaveValue('smtp.gmail.com')
    expect(screen.getByLabelText(ko.mail.port)).toHaveValue(587)
    expect(screen.getByLabelText(ko.mail.security)).toHaveValue('starttls')
    expect(screen.getByText(ko.mail.hintGmail)).toBeInTheDocument()
  })

  it('empty_fields_explain_values', async () => {
    renderMail(null)
    const username = await screen.findByLabelText(ko.mail.username)
    expect(username).toHaveAttribute('placeholder', ko.mail.usernamePlaceholder)
    expect(username).toHaveAccessibleDescription(ko.mail.usernameHint)
    expect(screen.getByLabelText(ko.mail.password)).toHaveAccessibleDescription(
      ko.mail.passwordHint,
    )
    expect(
      screen.getByLabelText(ko.mail.fromAddress),
    ).toHaveAccessibleDescription(ko.mail.fromAddressHint)
  })

  it('shows_saved_preset', async () => {
    renderMail(SAVED)
    const select = await screen.findByLabelText(ko.mail.service)
    expect(select).toHaveValue('naver')
    expect(screen.getByText(ko.mail.hintNaver)).toBeInTheDocument()
  })

  it('password_placeholder_when_set', async () => {
    let body: Record<string, unknown> | null = null
    const { user } = renderMail(SAVED, {
      'PUT /api/mail': async (req) => {
        body = (await req.json()) as Record<string, unknown>
        return json(200, { config: SAVED })
      },
    })
    const password = await screen.findByLabelText(ko.mail.password)
    expect(password).toHaveValue('')
    expect(password).toHaveAttribute('placeholder', ko.mail.passwordSet)
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    await screen.findByText(ko.mail.saved)
    expect(body).not.toBeNull()
    expect(body).not.toHaveProperty('password')
    expect(body).toMatchObject({
      host: 'smtp.naver.com',
      port: 587,
      security: 'starttls',
      username: 'me@naver.com',
      from_address: 'me@naver.com',
    })
  })

  it('password_cleared_after_save', async () => {
    const bodies: Record<string, unknown>[] = []
    const { user } = renderMail(SAVED, {
      'PUT /api/mail': async (req) => {
        bodies.push((await req.json()) as Record<string, unknown>)
        return json(200, { config: SAVED })
      },
    })
    const password = await screen.findByLabelText(ko.mail.password)
    await user.type(password, 'app-pw')
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    await waitFor(() => {
      expect(bodies).toHaveLength(1)
    })
    expect(bodies[0]).toMatchObject({ password: 'app-pw' })
    await waitFor(() => {
      expect(screen.getByLabelText(ko.mail.password)).toHaveValue('')
    })
    await user.click(screen.getByRole('button', { name: ko.common.save }))
    await waitFor(() => {
      expect(bodies).toHaveLength(2)
    })
    expect(bodies[1]).not.toHaveProperty('password')
  })

  it('security_switch_swaps_port', async () => {
    const { user } = renderMail(null)
    const security = await screen.findByLabelText(ko.mail.security)
    const port = screen.getByLabelText(ko.mail.port)
    await user.selectOptions(security, ko.mail.security_tls)
    expect(port).toHaveValue(465)
    await user.selectOptions(security, ko.mail.security_starttls)
    expect(port).toHaveValue(587)
    await user.clear(port)
    await user.type(port, '2525')
    await user.selectOptions(security, ko.mail.security_tls)
    expect(port).toHaveValue(2525)
    await user.selectOptions(security, ko.mail.security_none)
    await user.clear(port)
    await user.type(port, '587')
    await user.selectOptions(security, ko.mail.security_none)
    expect(port).toHaveValue(587)
  })

  it('none_security_warns', async () => {
    const { user } = renderMail(null)
    await user.selectOptions(
      await screen.findByLabelText(ko.mail.security),
      ko.mail.security_none,
    )
    expect(screen.getByText(ko.mail.noneWarn)).toBeInTheDocument()
  })

  it('test_send_defaults_to_own_email', async () => {
    let body: unknown = null
    const { user } = renderMail(
      SAVED,
      {
        'POST /api/mail/test': async (req) => {
          body = await req.json()
          return json(204, null)
        },
      },
      'owner@example.com',
    )
    const to = await screen.findByLabelText(ko.mail.testTo)
    expect(screen.getByText(ko.mail.testHint)).toBeInTheDocument()
    await waitFor(() => {
      expect(to).toHaveValue('owner@example.com')
    })
    await user.click(screen.getByRole('button', { name: ko.mail.testSend }))
    expect(await screen.findByText(ko.mail.testSent)).toBeInTheDocument()
    expect(body).toEqual({ to: 'owner@example.com' })
  })

  it('test_send_failure_shows_message', async () => {
    const { user } = renderMail(
      SAVED,
      {
        'POST /api/mail/test': () =>
          json(409, {
            error: 'mail.sendFailed',
            params: { message: '535 auth failed' },
          }),
      },
      'owner@example.com',
    )
    await screen.findByLabelText(ko.mail.testTo)
    await user.click(screen.getByRole('button', { name: ko.mail.testSend }))
    expect(
      await screen.findByText('메일을 보내지 못했어요. 535 auth failed'),
    ).toBeInTheDocument()
  })

  it('delete_settings', async () => {
    let deleted = 0
    const { user } = renderMail(SAVED, {
      'DELETE /api/mail': () => {
        deleted += 1
        return json(204, null)
      },
    })
    await user.click(
      await screen.findByRole('button', { name: ko.mail.delete }),
    )
    const dialog = await screen.findByRole('alertdialog')
    await user.click(
      within(dialog).getByRole('button', { name: ko.mail.deleteAction }),
    )
    await waitFor(() => {
      expect(deleted).toBe(1)
    })
    await waitFor(() => {
      expect(screen.getByLabelText(ko.mail.host)).toHaveValue('')
    })
    expect(screen.getByLabelText(ko.mail.service)).toHaveValue('custom')
    expect(screen.queryByRole('button', { name: ko.mail.delete })).toBeNull()
  })
})
