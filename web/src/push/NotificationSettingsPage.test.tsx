import { cleanup, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import { emptyFeedRoutes, settingsFixture, userFixture } from '@/test/fixtures'
import { fakeSub, installPush } from '@/test/push'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

const prefs = {
  upload: true,
  comment: true,
  mention: false,
  story: true,
  milestone: true,
}

function render(extra: Parameters<typeof mockFetch>[0] = {}) {
  const patches: unknown[] = []
  // 서버처럼 저장값 유지.
  let saved = { ...prefs }
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => json(200, { user: userFixture('owner') }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/push/prefs': () => json(200, saved),
    'PATCH /api/push/prefs': async (req) => {
      const body = (await req.json()) as Record<string, boolean>
      patches.push(body)
      saved = { ...saved, ...body }
      return json(200, saved)
    },
    'GET /api/push/subscription': () =>
      json(200, { endpoint: 'https://fcm.googleapis.com/a' }),
    'PUT /api/push/subscription': () => json(204, null),
    'DELETE /api/push/subscription': () => json(204, null),
    'GET /api/push/key': () => json(200, { public_key: 'AAAA' }),
    ...extra,
  })
  return { patches, ...renderApp('/me/notifications') }
}

const deviceSwitch = () =>
  screen.findByRole('switch', { name: ko.push.deviceToggle })

describe('NotificationSettingsPage', () => {
  it('settings_states', async () => {
    // unsupported
    render()
    expect(await deviceSwitch()).toBeDisabled()
    expect(await deviceSwitch()).toHaveAccessibleDescription(
      ko.push.unsupported,
    )
    cleanup()

    // ios-install
    vi.stubGlobal('navigator', {
      userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)',
      maxTouchPoints: 5,
    })
    render()
    expect(await deviceSwitch()).toBeDisabled()
    expect(screen.getByText(ko.push.iosInstall)).toBeInTheDocument()
    cleanup()
    vi.unstubAllGlobals()

    // denied
    installPush({ current: null, permission: 'denied' })
    render()
    expect(await deviceSwitch()).toBeDisabled()
    expect(await deviceSwitch()).toHaveAccessibleDescription(ko.push.denied)
    cleanup()
    vi.unstubAllGlobals()

    // supported
    installPush({ current: null })
    render()
    expect(await deviceSwitch()).toBeEnabled()
    expect(screen.queryByText(ko.push.unsupported)).not.toBeInTheDocument()
    expect(screen.queryByText(ko.push.denied)).not.toBeInTheDocument()
    expect(screen.queryByText(ko.push.iosInstall)).not.toBeInTheDocument()
    expect(await deviceSwitch()).not.toHaveAttribute('aria-describedby')
  })

  it('device_switch_disabled_while_checking', async () => {
    installPush({ current: fakeSub('https://fcm.googleapis.com/a') })
    let release: () => void = () => undefined
    render({
      'GET /api/push/subscription': () =>
        new Promise((resolve) => {
          release = () => {
            resolve(json(200, { endpoint: 'https://fcm.googleapis.com/a' }))
          }
        }),
    })
    const sw = await deviceSwitch()
    expect(sw).toBeDisabled()
    expect(sw).toHaveAttribute('aria-checked', 'false')
    release()
    await waitFor(() => {
      expect(sw).toBeEnabled()
      expect(sw).toHaveAttribute('aria-checked', 'true')
    })
  })

  it('device_toggle_calls_enable_disable', async () => {
    const sub = fakeSub('https://fcm.googleapis.com/a')
    const { subscribe } = installPush({ current: sub })
    const { user } = render()
    const sw = await deviceSwitch()
    await waitFor(() => {
      expect(sw).toHaveAttribute('aria-checked', 'true')
    })
    await user.click(sw)
    await waitFor(() => {
      expect(sub.unsubscribe).toHaveBeenCalled()
      expect(sw).toHaveAttribute('aria-checked', 'false')
    })
    await user.click(sw)
    await waitFor(() => {
      expect(subscribe).toHaveBeenCalled()
      expect(sw).toHaveAttribute('aria-checked', 'true')
    })
  })

  it('shows_denied_message_when_permission_refused', async () => {
    const { subscribe } = installPush({ current: null })
    const notification = Notification as unknown as {
      requestPermission: () => Promise<string>
      permission: string
    }
    notification.requestPermission = () => {
      notification.permission = 'denied'
      return Promise.resolve('denied')
    }
    const { user } = render()
    await user.click(await deviceSwitch())
    // 거부로 바뀌면 브라우저 설정 안내 하나만.
    expect(await screen.findByText(ko.push.denied)).toBeInTheDocument()
    expect(screen.queryByText(ko.push.permissionDenied)).not.toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(subscribe).not.toHaveBeenCalled()
  })

  it('shows_permission_message_when_prompt_dismissed', async () => {
    const { subscribe } = installPush({ current: null })
    const notification = Notification as unknown as {
      requestPermission: () => Promise<string>
    }
    notification.requestPermission = () => Promise.resolve('default')
    const { user } = render()
    await user.click(await deviceSwitch())
    expect(
      await screen.findByText(ko.push.permissionDenied),
    ).toBeInTheDocument()
    expect(screen.queryByText(ko.push.denied)).not.toBeInTheDocument()
    expect(await deviceSwitch()).toBeEnabled()
    expect(subscribe).not.toHaveBeenCalled()
  })

  it('kind_toggle_patches_and_rolls_back_on_error', async () => {
    installPush({ current: null })
    const { user, patches } = render()
    const mention = await screen.findByRole('switch', {
      name: ko.push.kind.mention,
    })
    await waitFor(() => {
      expect(mention).toHaveAttribute('aria-checked', 'false')
    })
    await user.click(mention)
    await waitFor(() => {
      expect(mention).toHaveAttribute('aria-checked', 'true')
    })
    expect(patches).toEqual([{ mention: true }])
    cleanup()

    installPush({ current: null })
    const r = render({
      'PATCH /api/push/prefs': () =>
        json(500, { error: 'server.internal', params: {} }),
    })
    const comment = await screen.findByRole('switch', {
      name: ko.push.kind.comment,
    })
    await waitFor(() => {
      expect(comment).toHaveAttribute('aria-checked', 'true')
    })
    await r.user.click(comment)
    expect(await screen.findByRole('alert')).toBeInTheDocument()
    await waitFor(() => {
      expect(comment).toHaveAttribute('aria-checked', 'true')
    })
  })

  // 앞 PATCH 응답이 늦게 와도 뒤 변경을 덮지 않음.
  it('kind_toggles_in_a_row_keep_both', async () => {
    installPush({ current: null })
    let server = { ...prefs }
    let releaseFirst: () => void = () => undefined
    let calls = 0
    const { user } = render({
      'GET /api/push/prefs': () => json(200, server),
      'PATCH /api/push/prefs': async (req) => {
        const body = (await req.json()) as Record<string, boolean>
        server = { ...server, ...body }
        const snapshot = { ...server }
        calls += 1
        if (calls > 1) return json(200, snapshot)
        return new Promise((resolve) => {
          releaseFirst = () => {
            resolve(json(200, snapshot))
          }
        })
      },
    })
    const mention = await screen.findByRole('switch', {
      name: ko.push.kind.mention,
    })
    const comment = screen.getByRole('switch', { name: ko.push.kind.comment })
    await waitFor(() => {
      expect(mention).toBeEnabled()
    })
    await user.click(mention)
    await user.click(comment)
    await waitFor(() => {
      expect(calls).toBe(2)
    })
    releaseFirst()
    await waitFor(() => {
      expect(mention).toHaveAttribute('aria-checked', 'true')
      expect(comment).toHaveAttribute('aria-checked', 'false')
    })
    // 마지막 응답 반영 뒤에도 유지.
    await new Promise((r) => setTimeout(r, 50))
    expect(mention).toHaveAttribute('aria-checked', 'true')
    expect(comment).toHaveAttribute('aria-checked', 'false')
  })
})
