import { screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { DayDetail } from '@/api/types'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import {
  babyFixture,
  dayFixture,
  settingsFixture,
  userFixture,
} from '@/test/fixtures'
import { intersectAll } from '@/test/intersection'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
  // 오늘 = 2025-11-20(서울). 올해 날짜 링크 이름은 연도 없음.
  vi.useFakeTimers({
    shouldAdvanceTime: true,
    now: new Date('2025-11-20T03:00:00Z'),
  })
})

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

const RECORDS = [
  '2025-09-02',
  '2025-10-27',
  '2025-10-28',
  '2025-10-30',
  '2025-11-02',
  '2025-12-30',
  '2026-01-02',
]

function detail(date: string, over: Partial<DayDetail>): DayDetail {
  return {
    date,
    photo_count: 1,
    video_count: 0,
    uploader_ids: [],
    baby_ids: [],
    like_count: 0,
    comment_count: 0,
    prev_date: null,
    next_date: null,
    first_date: '2025-09-02',
    last_date: '2025-11-02',
    stories: [],
    milestones: [],
    ...over,
  }
}

function renderStrip(opts: { path: string; bounds?: Partial<DayDetail> }) {
  const daysRequests: URL[] = []
  const dayRoutes = Object.fromEntries(
    RECORDS.map((d) => [
      `GET /api/days/${d}`,
      () => json(200, detail(d, opts.bounds ?? {})),
    ]),
  )
  mockFetch({
    'GET /api/notifications/unread-count': () => json(200, { count: 0 }),
    'GET /api/auth/me': () => json(200, { user: userFixture('guardian') }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/babies': () => json(200, [babyFixture()]),
    'GET /api/users': () => json(200, { users: [userFixture('guardian')] }),
    'GET /api/assets': () => json(200, { items: [], next_cursor: null }),
    'GET /api/memories': () => json(200, null),
    'GET /api/days': (req) => {
      const url = new URL(req.url)
      daysRequests.push(url)
      const from = url.searchParams.get('from') ?? '0000-00-00'
      const to = url.searchParams.get('to') ?? '9999-99-99'
      const items = RECORDS.filter((d) => d >= from && d <= to)
        .reverse()
        .map((d) => dayFixture(d))
      return json(200, { items, next_cursor: null })
    },
    ...dayRoutes,
  })
  return { ...renderApp(opts.path), daysRequests }
}

async function findStrip() {
  return within(await screen.findByRole('navigation', { name: ko.day.nav }))
}

describe('DayStrip', () => {
  it('current_day_marked', async () => {
    renderStrip({ path: '/days/2025-10-28' })
    const strip = await findStrip()

    const current = strip.getByRole('link', { name: '10월 28일 화요일' })
    expect(current).toHaveAttribute('aria-current', 'date')
    const other = await strip.findByRole('link', { name: '10월 27일 월요일' })
    expect(other).not.toHaveAttribute('aria-current')
  })

  it('record_days_are_links_empty_days_are_not', async () => {
    renderStrip({ path: '/days/2025-10-28' })
    const strip = await findStrip()

    expect(
      await strip.findByRole('link', { name: '10월 27일 월요일' }),
    ).toHaveAttribute('href', '/days/2025-10-27')
    expect(strip.getByRole('link', { name: '9월 2일 화요일' })).toHaveAttribute(
      'href',
      '/days/2025-09-02',
    )
    // 기록 없는 날은 칸만 있고 링크 아님, 스크린 리더 숨김.
    expect(strip.queryByRole('link', { name: '10월 29일 수요일' })).toBeNull()
    const empty = document.querySelector('[data-date="2025-10-29"]')
    expect(empty).not.toBeNull()
    expect(empty?.closest('[aria-hidden="true"]')).not.toBeNull()
    // first_date ~ last_date 모든 날.
    expect(document.querySelector('[data-date="2025-09-02"]')).not.toBeNull()
    expect(document.querySelector('[data-date="2025-09-01"]')).toBeNull()
    expect(document.querySelector('[data-date="2025-11-02"]')).not.toBeNull()
    expect(document.querySelector('[data-date="2025-11-03"]')).toBeNull()
  })

  it('month_change_label', async () => {
    renderStrip({ path: '/days/2025-10-28' })
    const strip = await findStrip()

    expect(strip.getByText('9월')).toBeInTheDocument()
    expect(strip.getByText('10월')).toBeInTheDocument()
    expect(strip.getByText('11월')).toBeInTheDocument()
    // 달 표시는 날짜 칸 앞.
    const pill = strip.getByText('11월')
    const first = document.querySelector('[data-date="2025-11-01"]')
    expect(first).not.toBeNull()
    expect(
      pill.compareDocumentPosition(first as Node) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0)
  })

  it('year_change_label', async () => {
    renderStrip({
      path: '/days/2025-12-30',
      bounds: { first_date: '2025-12-30', last_date: '2026-01-02' },
    })
    const strip = await findStrip()

    expect(strip.getByText('12월')).toBeInTheDocument()
    expect(strip.getByText('2026년 1월')).toBeInTheDocument()
    // 올해가 아닌 날은 연도까지.
    expect(
      await strip.findByRole('link', { name: '2026년 1월 2일 금요일' }),
    ).toBeInTheDocument()
  })

  it('strip_navigation_replaces_history', async () => {
    const { router, user } = renderStrip({ path: '/' })
    await screen.findAllByText('우리 가족')
    await router.navigate({
      to: '/days/$date',
      params: { date: '2025-10-28' },
    })
    const strip = await findStrip()

    await user.click(
      await strip.findByRole('link', { name: '10월 27일 월요일' }),
    )
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/days/2025-10-27')
    })
    expect(
      strip.getByRole('link', { name: '10월 27일 월요일' }),
    ).toHaveAttribute('aria-current', 'date')
    await user.click(strip.getByRole('link', { name: '10월 30일 목요일' }))
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/days/2025-10-30')
    })

    // 띠 이동은 기록 대체. 뒤로 가기는 들어오기 전 화면.
    await user.click(screen.getByRole('button', { name: ko.day.back }))
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/')
    })
  })

  it('keyboard_focus_stays_on_new_current_day', async () => {
    const { router, user } = renderStrip({ path: '/days/2025-10-28' })
    const strip = await findStrip()
    const link = await strip.findByRole('link', { name: '10월 27일 월요일' })

    link.focus()
    await user.keyboard('{Enter}')

    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/days/2025-10-27')
    })
    // 같은 칸 요소 유지. 포커스가 body 로 빠지지 않음.
    const current = strip.getByRole('link', { name: '10월 27일 월요일' })
    expect(current).toHaveAttribute('aria-current', 'date')
    expect(document.activeElement).toBe(current)
  })

  it('single_tab_stop_on_current_day', async () => {
    const { user } = renderStrip({ path: '/days/2025-10-28' })
    const strip = await findStrip()
    const current = strip.getByRole('link', { name: '10월 28일 화요일' })
    const other = await strip.findByRole('link', { name: '10월 27일 월요일' })
    expect(current).toHaveAttribute('tabindex', '0')
    expect(other).toHaveAttribute('tabindex', '-1')

    // 띠 앞 요소에서 Tab 한 번에 지금 날, Shift+Tab 한 번에 띠 밖.
    const back = screen.getByRole('button', { name: ko.day.back })
    back.focus()
    await user.tab()
    expect(document.activeElement).toBe(current)
    await user.tab({ shift: true })
    expect(document.activeElement).toBe(back)
  })

  it('arrow_keys_move_between_record_days', async () => {
    const { router, user } = renderStrip({ path: '/days/2025-10-28' })
    const strip = await findStrip()
    await strip.findByRole('link', { name: '9월 2일 화요일' })
    await strip.findByRole('link', { name: '11월 2일 일요일' })
    const focused = () => document.activeElement?.getAttribute('aria-label')

    strip.getByRole('link', { name: '10월 28일 화요일' }).focus()
    // 기록 없는 29일 건너뜀.
    await user.keyboard('{ArrowRight}')
    expect(focused()).toBe('10월 30일 목요일')
    await user.keyboard('{ArrowLeft}{ArrowLeft}')
    expect(focused()).toBe('10월 27일 월요일')
    await user.keyboard('{Home}')
    expect(focused()).toBe('9월 2일 화요일')
    await user.keyboard('{End}')
    expect(focused()).toBe('11월 2일 일요일')
    // 포커스 이동만, 날 이동 없음.
    expect(router.state.location.pathname).toBe('/days/2025-10-28')
  })

  it('strip_passes_baby_filter', async () => {
    const { daysRequests } = renderStrip({ path: '/days/2025-10-28?baby=b' })
    const strip = await findStrip()

    expect(
      await strip.findByRole('link', { name: '10월 27일 월요일' }),
    ).toHaveAttribute('href', '/days/2025-10-27?baby=b')
    expect(daysRequests.length).toBeGreaterThan(0)
    for (const url of daysRequests) {
      expect(url.searchParams.get('baby_id')).toBe('b')
    }
  })

  it('loads_more_near_edge', async () => {
    const { daysRequests } = renderStrip({
      path: '/days/2025-10-28',
      bounds: { first_date: '2025-01-01', last_date: '2025-11-02' },
    })
    const strip = await findStrip()
    await strip.findByRole('link', { name: '10월 27일 월요일' })

    // 처음엔 지금 날 앞뒤 달만.
    const froms = () => daysRequests.map((u) => u.searchParams.get('from'))
    expect(froms().sort()).toEqual(['2025-09-01', '2025-10-01', '2025-11-01'])
    expect(document.querySelector('[data-date="2025-08-31"]')).toBeNull()

    intersectAll()

    await waitFor(() => {
      expect(froms()).toContain('2025-08-01')
    })
    expect(document.querySelector('[data-date="2025-08-01"]')).not.toBeNull()
    // 뒤쪽은 마지막 날 달까지라 더 없음.
    expect(froms()).not.toContain('2025-12-01')
  })

  it('range_clamped_to_bounds', async () => {
    // 범위 응답이 지금 날을 벗어나도(갱신 전 잠깐) 빈 띠 대신 가까운 끝 달부터.
    renderStrip({
      path: '/days/2025-10-28',
      bounds: { first_date: '2025-12-30', last_date: '2026-01-02' },
    })
    await findStrip()

    await waitFor(() => {
      expect(document.querySelectorAll('[data-date]')).toHaveLength(4)
    })
  })

  it('single_day_strip', async () => {
    renderStrip({
      path: '/days/2025-10-28',
      bounds: { first_date: '2025-10-28', last_date: '2025-10-28' },
    })
    const strip = await findStrip()

    await waitFor(() => {
      expect(document.querySelectorAll('[data-date]')).toHaveLength(1)
    })
    expect(strip.getAllByRole('link')).toHaveLength(1)
    expect(
      strip.getByRole('button', { name: ko.day.stripEarlier }),
    ).toBeDisabled()
    expect(
      strip.getByRole('button', { name: ko.day.stripLater }),
    ).toBeDisabled()
  })
})
