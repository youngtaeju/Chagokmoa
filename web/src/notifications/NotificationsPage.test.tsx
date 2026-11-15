import { screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { AppNotification } from '@/api/types'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import {
  assetFixture,
  assetSummaryFixture,
  emptyFeedRoutes,
  notificationFixture,
  settingsFixture,
  userFixture,
} from '@/test/fixtures'
import { intersectAll } from '@/test/intersection'
import { renderApp } from '@/test/router'
import { unreadKey } from './useUnreadCount'

const me = userFixture('guardian')
const mom = {
  ...userFixture('family'),
  id: '00000000-0000-4000-8000-000000000002',
  display_name: '엄마',
}
const story = { id: 's', excerpt: '오늘 처음 웃었어요', cover: null }

type Page = {
  items: AppNotification[]
  next_cursor: string | null
  seen_at: string
}

beforeEach(() => {
  initI18n('ko')
  // 2026-11-16 12:00 KST. 타이머는 실제.
  vi.useFakeTimers({ toFake: ['Date'], now: new Date('2026-11-16T03:00:00Z') })
  // jsdom 은 canvas 미구현. blurhash 경고 방지.
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(
    () => null,
  )
})

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

function renderNotifications(
  page: (url: URL) => Page,
  extra: Parameters<typeof mockFetch>[0] = {},
  path = '/notifications',
) {
  const requests: URL[] = []
  const state = { seen: 0 }
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => json(200, { user: me }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/users': () => json(200, { users: [me, mom] }),
    'GET /api/notifications/unread-count': () =>
      json(200, { count: state.seen > 0 ? 0 : 5 }),
    'POST /api/notifications/seen': () => {
      state.seen += 1
      return new Response(null, { status: 204 })
    },
    'GET /api/notifications': (req) => {
      const url = new URL(req.url)
      requests.push(url)
      return json(200, page(url))
    },
    ...extra,
  })
  return { ...renderApp(path), requests, state }
}

function rowOf(text: string): HTMLElement {
  const row = screen.getByText(text).closest('[data-new]')
  if (!(row instanceof HTMLElement)) throw new Error(`줄 없음: ${text}`)
  return row
}

describe('NotificationsPage', () => {
  it('renders_sections_and_sentences', async () => {
    renderNotifications(() => ({
      items: [
        notificationFixture('n1', {
          count: 12,
          updated_at: '2026-11-16T02:00:00Z',
        }),
        notificationFixture('n2', {
          kind: 'mention',
          comment: { excerpt: `<@${me.id}> 이것 봐요` },
          updated_at: '2026-11-16T01:00:00Z',
        }),
        notificationFixture('n3', {
          kind: 'comment',
          asset: null,
          story,
          comment: { excerpt: '귀여워요' },
          updated_at: '2026-11-15T10:00:00Z',
        }),
        notificationFixture('n4', {
          kind: 'milestone',
          asset: null,
          milestone: {
            id: 'm',
            preset_key: 'first_tooth',
            custom_label: null,
            cover: null,
          },
          updated_at: '2026-11-13T10:00:00Z',
        }),
        notificationFixture('n5', {
          kind: 'story',
          asset: null,
          story,
          updated_at: '2026-11-01T10:00:00Z',
        }),
        notificationFixture('n6', {
          kind: 'comment',
          asset: assetSummaryFixture('b', { uploaded_by: mom.id }),
          comment: { excerpt: '예뻐요' },
          updated_at: '2026-10-01T10:00:00Z',
        }),
      ],
      next_cursor: null,
      seen_at: '2026-11-16T00:00:00Z',
    }))
    expect(
      await screen.findByRole('heading', {
        level: 1,
        name: ko.notifications.title,
      }),
    ).toBeVisible()
    const today = (await screen.findByRole('heading', { name: '오늘' }))
      .parentElement
    const week = screen.getByRole('heading', { name: '이번 주' }).parentElement
    const earlier = screen.getByRole('heading', { name: '이전' }).parentElement
    if (!today || !week || !earlier) throw new Error('묶음 없음')

    expect(
      within(today).getByText('엄마님이 사진 12장을 올렸어요'),
    ).toBeInTheDocument()
    // 멘션 토큰은 현재 이름.
    expect(
      await within(today).findByText(
        '엄마님이 회원님을 언급했어요: "@guardian 이것 봐요"',
      ),
    ).toBeInTheDocument()
    expect(
      within(week).getByText('엄마님이 이야기에 댓글을 남겼어요: "귀여워요"'),
    ).toBeInTheDocument()
    expect(
      within(week).getByText('엄마님이 마일스톤을 남겼어요: 첫 이'),
    ).toBeInTheDocument()
    expect(
      within(earlier).getByText(
        '엄마님이 이야기를 남겼어요: "오늘 처음 웃었어요"',
      ),
    ).toBeInTheDocument()
    // 내 사진이 아닌 사진 댓글.
    expect(
      within(earlier).getByText('엄마님이 댓글을 남겼어요: "예뻐요"'),
    ).toBeInTheDocument()
    // 사진 썸네일. 대상 사진 없는 줄은 생략.
    expect(
      rowOf('엄마님이 사진 12장을 올렸어요').querySelector('img'),
    ).toHaveAttribute('src', '/api/assets/a/thumb.webp')
    expect(
      rowOf('엄마님이 마일스톤을 남겼어요: 첫 이').querySelector('img'),
    ).toBeNull()
  })

  it('upload_one_and_my_photo_comment_sentences', async () => {
    renderNotifications(() => ({
      items: [
        notificationFixture('n1'),
        notificationFixture('n2', {
          kind: 'comment',
          comment: { excerpt: '예뻐요' },
        }),
      ],
      next_cursor: null,
      seen_at: '2026-11-16T00:00:00Z',
    }))
    expect(
      await screen.findByText('엄마님이 사진을 올렸어요'),
    ).toBeInTheDocument()
    expect(
      screen.getByText('엄마님이 회원님 사진에 댓글을 남겼어요: "예뻐요"'),
    ).toBeInTheDocument()
  })

  it('highlights_rows_newer_than_seen_at', async () => {
    renderNotifications(() => ({
      items: [
        notificationFixture('n1', {
          count: 2,
          updated_at: '2026-11-16T02:00:00Z',
        }),
        notificationFixture('n2', { updated_at: '2026-11-15T20:00:00Z' }),
      ],
      next_cursor: null,
      seen_at: '2026-11-16T00:00:00Z',
    }))
    await screen.findByText('엄마님이 사진 2장을 올렸어요')
    expect(rowOf('엄마님이 사진 2장을 올렸어요')).toHaveAttribute(
      'data-new',
      'true',
    )
    expect(rowOf('엄마님이 사진을 올렸어요')).toHaveAttribute(
      'data-new',
      'false',
    )
    // 화면 낭독기용 새 알림 표시.
    expect(
      screen.getByRole('link', { name: /엄마님이 사진 2장을 올렸어요/ }),
    ).toHaveAccessibleName(/새 알림/)
    expect(
      screen.getByRole('link', { name: /엄마님이 사진을 올렸어요/ }),
    ).not.toHaveAccessibleName(/새 알림/)
  })

  it('truncated_mention_token_hidden', async () => {
    renderNotifications(() => ({
      items: [
        notificationFixture('n1', {
          kind: 'mention',
          // 서버 100자 자르기로 끝 토큰이 잘린 발췌.
          comment: { excerpt: `<@${me.id}> 봐요 <@0000000` },
        }),
      ],
      next_cursor: null,
      seen_at: '2026-11-16T00:00:00Z',
    }))
    expect(
      await screen.findByText(/엄마님이 회원님을 언급했어요: "@guardian 봐요/),
    ).toBeInTheDocument()
    expect(screen.queryByText(/<@/)).toBeNull()
  })

  it('highlight_threshold_fixed_to_first_page', async () => {
    const { requests, state } = renderNotifications((url) =>
      url.searchParams.get('cursor') === 'c1'
        ? {
            items: [
              notificationFixture('n3', {
                count: 3,
                updated_at: '2026-11-16T01:00:00Z',
              }),
              notificationFixture('n4', {
                count: 4,
                updated_at: '2026-11-15T20:00:00Z',
              }),
            ],
            next_cursor: null,
            // 첫 페이지 뒤 seen 호출로 더 새로운 값.
            seen_at: '2026-11-16T03:00:00Z',
          }
        : {
            items: [
              notificationFixture('n1', {
                count: 2,
                updated_at: '2026-11-16T02:30:00Z',
              }),
            ],
            next_cursor: 'c1',
            seen_at: '2026-11-16T00:00:00Z',
          },
    )
    await screen.findByText('엄마님이 사진 2장을 올렸어요')
    await waitFor(() => {
      expect(state.seen).toBe(1)
    })
    intersectAll()
    expect(
      await screen.findByText('엄마님이 사진 3장을 올렸어요'),
    ).toBeInTheDocument()
    expect(rowOf('엄마님이 사진 3장을 올렸어요')).toHaveAttribute(
      'data-new',
      'true',
    )
    expect(rowOf('엄마님이 사진 4장을 올렸어요')).toHaveAttribute(
      'data-new',
      'false',
    )
    expect(requests.map((u) => u.searchParams.get('cursor'))).toEqual([
      null,
      'c1',
    ])
    expect(state.seen).toBe(1)
  })

  it('seen_posts_and_clears_badge', async () => {
    const { state, queryClient } = renderNotifications(() => ({
      items: [
        notificationFixture('n1', { updated_at: '2026-11-16T02:00:00Z' }),
      ],
      next_cursor: null,
      seen_at: '2026-11-16T00:00:00Z',
    }))
    await screen.findByText('엄마님이 사진을 올렸어요')
    await waitFor(() => {
      expect(queryClient.getQueryData(unreadKey)).toEqual({ count: 0 })
    })
    expect(state.seen).toBe(1)
    // 이미 받은 목록의 강조는 유지.
    expect(rowOf('엄마님이 사진을 올렸어요')).toHaveAttribute(
      'data-new',
      'true',
    )
  })

  it('empty_state', async () => {
    renderNotifications(() => ({
      items: [],
      next_cursor: null,
      seen_at: '2026-11-16T00:00:00Z',
    }))
    expect(await screen.findByText(ko.notifications.empty)).toBeInTheDocument()
  })

  it('row_click_navigates', async () => {
    const { user, router } = renderNotifications(
      () => ({
        items: [
          notificationFixture('n1', {
            kind: 'comment',
            comment: { excerpt: '예뻐요' },
          }),
        ],
        next_cursor: null,
        seen_at: '2026-11-16T00:00:00Z',
      }),
      {
        'GET /api/babies': () => json(200, []),
        'GET /api/assets': () =>
          json(200, { items: [assetSummaryFixture('a')], next_cursor: null }),
        'GET /api/assets/a': () => json(200, assetFixture('a')),
        'GET /api/assets/a/comments': () =>
          json(200, { items: [], next_cursor: null }),
      },
    )
    await user.click(
      await screen.findByRole('link', {
        name: /엄마님이 회원님 사진에 댓글을 남겼어요/,
      }),
    )
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/photos/a')
    })
    expect(router.state.location.search).toMatchObject({ panel: 'comments' })
  })

  it('back_goes_home_on_direct_entry', async () => {
    const { user, router } = renderNotifications(() => ({
      items: [],
      next_cursor: null,
      seen_at: '2026-11-16T00:00:00Z',
    }))
    await user.click(await screen.findByRole('button', { name: ko.me.back }))
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/')
    })
  })
})
