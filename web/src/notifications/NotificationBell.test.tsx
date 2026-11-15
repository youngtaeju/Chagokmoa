import { cleanup, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import {
  babyFixture,
  emptyFeedRoutes,
  settingsFixture,
  userFixture,
} from '@/test/fixtures'
import { renderApp } from '@/test/router'
import { formatBadge } from './useUnreadCount'

beforeEach(() => {
  initI18n('ko')
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

function renderHome(count: number) {
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => json(200, { user: userFixture('family') }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/babies': () => json(200, [babyFixture()]),
    'GET /api/notifications/unread-count': () => json(200, { count }),
  })
  return renderApp('/')
}

// 사이드바 줄과 구분: 종만 aria-label 로 이름을 가짐.
async function findBell(name: string) {
  const links = await screen.findAllByRole('link', { name: new RegExp(name) })
  const bell = links.find((l) => l.hasAttribute('aria-label'))
  if (!bell) throw new Error('종 없음')
  return bell
}

describe('formatBadge', () => {
  it.each([
    [0, null],
    [7, '7'],
    [99, '99'],
    [100, '99+'],
  ] as const)('%s -> %s', (n, expected) => {
    expect(formatBadge(n)).toBe(expected)
  })
})

describe('NotificationBell', () => {
  it('bell_shows_count', async () => {
    renderHome(3)
    const link = await findBell('새 알림 3개')
    expect(link).toHaveAttribute('href', '/notifications')
    expect(link).toHaveTextContent('3')
  })

  it('bell_without_badge_when_zero', async () => {
    renderHome(0)
    const link = await findBell(ko.notifications.title)
    expect(link).toHaveTextContent('')
  })
})
