import { cleanup, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import { emptyFeedRoutes, settingsFixture, userFixture } from '@/test/fixtures'
import { fakeSub, installPush } from '@/test/push'
import { renderApp } from '@/test/router'

const KEY = 'cgm.push.banner.dismissed'

beforeEach(() => {
  initI18n('ko')
  localStorage.clear()
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

function render(extra: Parameters<typeof mockFetch>[0] = {}) {
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => json(200, { user: userFixture('owner') }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/users': () => json(200, { users: [] }),
    'GET /api/notifications/unread-count': () => json(200, { count: 0 }),
    'GET /api/notifications': () =>
      json(200, { items: [], next_cursor: null, seen_at: null }),
    'POST /api/notifications/seen': () => new Response(null, { status: 204 }),
    'GET /api/push/subscription': () => json(200, { endpoint: null }),
    'PUT /api/push/subscription': () => json(204, null),
    'GET /api/push/key': () => json(200, { public_key: 'AAAA' }),
    ...extra,
  })
  return renderApp('/notifications')
}

describe('PushBanner', () => {
  it('banner_shows_only_when_can_enable', async () => {
    render()
    await screen.findByText(ko.notifications.empty)
    expect(screen.queryByText(ko.push.banner)).not.toBeInTheDocument()
    cleanup()

    installPush({ current: null, permission: 'denied' })
    render()
    await screen.findByText(ko.notifications.empty)
    expect(screen.queryByText(ko.push.banner)).not.toBeInTheDocument()
    cleanup()
    vi.unstubAllGlobals()

    installPush({ current: null })
    localStorage.setItem(KEY, '1')
    render()
    await screen.findByText(ko.notifications.empty)
    expect(screen.queryByText(ko.push.banner)).not.toBeInTheDocument()
    cleanup()

    localStorage.clear()
    render()
    expect(await screen.findByText(ko.push.banner)).toBeInTheDocument()
  })

  it('banner_hidden_while_checking_device', async () => {
    const sub = fakeSub('https://fcm.googleapis.com/a')
    installPush({ current: sub })
    let release: () => void = () => undefined
    render({
      'GET /api/push/subscription': () =>
        new Promise((resolve) => {
          release = () => {
            resolve(json(200, { endpoint: null }))
          }
        }),
    })
    await screen.findByText(ko.notifications.empty)
    expect(screen.queryByText(ko.push.banner)).not.toBeInTheDocument()
    // 서버에 이 세션 구독 없음 → 브라우저 구독 해지, 꺼짐 확정 후 띠 표시.
    release()
    expect(await screen.findByText(ko.push.banner)).toBeInTheDocument()
    expect(sub.unsubscribe).toHaveBeenCalled()
  })

  it('banner_dismiss_persists', async () => {
    installPush({ current: null })
    const { user } = render()
    await screen.findByText(ko.push.banner)
    await user.click(screen.getByRole('button', { name: ko.push.bannerClose }))
    expect(screen.queryByText(ko.push.banner)).not.toBeInTheDocument()
    expect(localStorage.getItem(KEY)).toBe('1')
  })

  it('banner_turn_on_hides', async () => {
    const { subscribe } = installPush({ current: null })
    const { user } = render()
    await screen.findByText(ko.push.banner)
    await user.click(screen.getByRole('button', { name: ko.push.bannerOn }))
    await waitFor(() => {
      expect(subscribe).toHaveBeenCalled()
      expect(screen.queryByText(ko.push.banner)).not.toBeInTheDocument()
    })
  })
})
