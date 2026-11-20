import { cleanup, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Role } from '@/api/types'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import {
  apiError,
  assetFixture,
  babyFixture,
  backupStatusFixture,
  emptyFeedRoutes,
  settingsFixture,
  userFixture,
} from '@/test/fixtures'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
})

afterEach(() => {
  localStorage.clear()
  vi.restoreAllMocks()
})

function mockApp(role: Role) {
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => json(200, { user: userFixture(role) }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/auth/sessions': () => json(200, { sessions: [] }),
    'GET /api/users': () => json(200, { users: [userFixture(role)] }),
    'GET /api/invites': () => json(200, { invites: [] }),
    'GET /api/trash': () => json(200, { items: [], next_cursor: null }),
    'GET /api/favorites': () => json(200, { items: [], next_cursor: null }),
    'GET /api/assets': () => json(200, { items: [], next_cursor: null }),
    'GET /api/assets/a1': () => json(200, assetFixture('a1')),
    'GET /api/backup': () => json(200, backupStatusFixture()),
    'GET /api/derivatives/regenerate': () =>
      json(200, { remaining: 0, failed: 0 }),
  })
}

describe('router', () => {
  it.each([
    ['/photo/a1?baby=b1', '/photos/a1?baby=b1'],
    ['/members', '/family'],
    ['/upload', '/'],
    ['/trash', '/settings/trash'],
    ['/me', '/settings'],
    ['/me/profile', '/settings/profile'],
    ['/me/trash', '/settings/trash'],
    ['/me/backup', '/settings/backup'],
    ['/me/instance', '/settings/general'],
  ])('legacy_me_paths_redirect %s', async (from, to) => {
    mockApp('owner')
    const { router } = renderApp('/')
    await screen.findAllByText(settingsFixture.instance_name)
    const before = router.history.length

    await router.navigate({ href: from })
    await waitFor(() => {
      expect(router.state.location.href).toBe(to)
    })
    // 예전 주소는 기록에 남지 않음. 이동 1회분만 증가.
    expect(router.history.length).toBe(before + 1)
    router.history.back()
    await waitFor(() => {
      expect(router.state.location.href).toBe('/')
    })
  })

  // 즐겨찾기는 앨범 탭으로 이동. 예전 주소는 교체 이동.
  it('legacy_favorites_redirect', async () => {
    mockApp('family')
    const { router } = renderApp('/me/favorites')
    expect(
      await screen.findByRole('heading', {
        level: 1,
        name: ko.albums.favorites,
      }),
    ).toBeInTheDocument()
    expect(router.state.location.href).toBe('/albums/favorites')
    expect(router.history.length).toBe(1)
  })

  it('legacy_path_direct_entry_replaced', async () => {
    mockApp('owner')
    const { router } = renderApp('/me')
    await waitFor(() => {
      expect(router.state.location.href).toBe('/settings')
    })
    expect(router.history.length).toBe(1)
  })

  it('family_route_open_to_all', async () => {
    mockApp('family')
    const { router } = renderApp('/family')
    expect(
      await screen.findByRole('heading', { name: ko.members.usersTitle }),
    ).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/family')
    // 초대·역할 변경은 관리자만.
    expect(
      screen.queryByRole('button', { name: ko.members.createInvite }),
    ).not.toBeInTheDocument()
  })

  it('settings_trash_guardian_only', async () => {
    mockApp('guardian')
    const { router } = renderApp('/settings/trash')
    expect(
      await screen.findByRole('heading', { name: ko.nav.trash }),
    ).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/settings/trash')
  })

  // 마지막 본 아기 없으면 대표 아기. 있으면 마지막 본 아기.
  it('babies_redirects_primary', async () => {
    const SKY = babyFixture()
    const SEA = babyFixture({
      id: '00000000-0000-4000-8000-0000000000b2',
      name: '바다',
      sort_order: 1,
    })
    mockFetch({
      'GET /api/notifications/unread-count': () => json(200, { count: 0 }),
      'GET /api/auth/me': () => json(200, { user: userFixture('family') }),
      'GET /api/settings': () =>
        json(200, { ...settingsFixture, primary_baby_id: SEA.id }),
      'GET /api/babies': () => json(200, [SKY, SEA]),
      ...Object.fromEntries(
        [SKY, SEA].flatMap((b) => [
          [
            `GET /api/babies/${b.id}/stats`,
            () =>
              json(200, {
                photo_count: 0,
                video_count: 0,
                day_count: 0,
                age_months: [],
              }),
          ],
          [`GET /api/babies/${b.id}/growth`, () => json(200, [])],
          [`GET /api/babies/${b.id}/milestones`, () => json(200, [])],
        ]),
      ),
    })
    const first = renderApp('/babies')
    expect(
      await screen.findByRole('heading', { level: 1, name: SEA.name }),
    ).toBeInTheDocument()
    expect(first.router.state.location.pathname).toBe(`/babies/${SEA.id}`)
    expect(first.router.history.length).toBe(1)
    cleanup()

    localStorage.setItem('cgm.baby.last', SKY.id)
    const second = renderApp('/babies')
    expect(
      await screen.findByRole('heading', { level: 1, name: SKY.name }),
    ).toBeInTheDocument()
    expect(second.router.state.location.pathname).toBe(`/babies/${SKY.id}`)
  })

  it('login_redirect_keeps_new_path', async () => {
    mockFetch({
      'GET /api/auth/me': () => apiError(401, 'auth.required'),
      'GET /api/auth/status': () => json(200, { signup_open: false }),
    })
    const { router } = renderApp('/settings/trash')
    await waitFor(() => {
      expect(router.state.location.href).toBe(
        '/login?redirect=%2Fsettings%2Ftrash',
      )
    })
  })
})
