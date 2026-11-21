import { act, cleanup, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Role, SessionInfo, Settings } from '@/api/types'
import { initI18n } from '@/i18n'
import { json, mockFetch } from '@/test/fetch'
import {
  apiError,
  babyFixture,
  emptyFeedRoutes,
  settingsFixture,
  userFixture,
} from '@/test/fixtures'
import { REGEN_POLL_MS } from '@/backup/RegenSection'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
})

afterEach(cleanup)

function session(over: Partial<SessionInfo>): SessionInfo {
  return {
    id: '00000000-0000-4000-8000-0000000000e1',
    created_at: '2026-01-01T00:00:00Z',
    last_seen_at: '2026-01-02T00:00:00Z',
    current: false,
    user_agent: 'Firefox',
    ...over,
  }
}

const CURRENT = session({
  id: '00000000-0000-4000-8000-0000000000e0',
  current: true,
  user_agent: 'Chrome',
})
const OTHER = session({})

function renderPage(
  extra: Parameters<typeof mockFetch>[0] = {},
  {
    role = 'guardian',
    settings = settingsFixture,
    path = '/settings/profile',
  }: { role?: Role; settings?: Settings; path?: string } = {},
) {
  mockFetch({
    'GET /api/auth/me': () => json(200, { user: userFixture(role) }),
    'GET /api/settings': () => json(200, settings),
    'GET /api/auth/sessions': () => json(200, { sessions: [CURRENT, OTHER] }),
    'GET /api/auth/login-method': () =>
      json(200, { has_password: true, oidc: 'none' }),
    ...emptyFeedRoutes,
    ...extra,
  })
  return renderApp(path)
}

describe('settings sub pages', () => {
  it.each([
    ['/settings/profile', '내 계정 프로필'],
    ['/settings/password', '내 계정 비밀번호 바꾸기'],
    ['/settings/devices', '내 계정 로그인한 기기'],
    ['/settings/appearance', '앱 화면과 언어'],
  ])('title_has_section %s', async (path, name) => {
    renderPage({}, { path })
    expect(
      await screen.findByRole('heading', { level: 1, name }),
    ).toBeInTheDocument()
  })

  it('wrong_current_password_stays_logged_in', async () => {
    const { user, router } = renderPage(
      {
        'POST /api/users/me/password': () =>
          apiError(401, 'auth.invalidCredentials'),
      },
      { path: '/settings/password' },
    )
    await user.type(await screen.findByLabelText('현재 비밀번호'), 'wrong-pass')
    await user.type(screen.getByLabelText('새 비밀번호'), 'new-password-1')
    await user.click(screen.getByRole('button', { name: '비밀번호 바꾸기' }))

    expect(
      await screen.findByText('현재 비밀번호가 맞지 않아요.'),
    ).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/settings/password')
  })

  it('password_change_invalidates_sessions', async () => {
    const { user, queryClient } = renderPage(
      { 'POST /api/users/me/password': () => json(204, null) },
      { path: '/settings/password' },
    )
    // 서버가 다른 세션을 지우므로 목록 캐시 무효화.
    queryClient.setQueryData(['sessions'], { sessions: [CURRENT, OTHER] })
    await user.type(
      await screen.findByLabelText('현재 비밀번호'),
      'old-password-1',
    )
    await user.type(screen.getByLabelText('새 비밀번호'), 'new-password-1')
    await user.click(screen.getByRole('button', { name: '비밀번호 바꾸기' }))
    await waitFor(() => {
      expect(queryClient.getQueryState(['sessions'])?.isInvalidated).toBe(true)
    })
  })

  it('passwordless_shows_create', async () => {
    const bodies: unknown[] = []
    const { user } = renderPage(
      {
        'GET /api/auth/login-method': () =>
          json(200, { has_password: false, oidc: 'linked' }),
        'POST /api/users/me/password': async (req) => {
          bodies.push(await req.json())
          return json(204, null)
        },
      },
      { path: '/settings/password' },
    )
    await user.type(
      await screen.findByLabelText('새 비밀번호'),
      'new-password-1',
    )
    expect(screen.queryByLabelText('현재 비밀번호')).not.toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: '비밀번호 바꾸기' }),
    ).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '비밀번호 만들기' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ new_password: 'new-password-1' }])
    })
  })

  it('password_section_retry', async () => {
    let calls = 0
    const { user } = renderPage(
      {
        'GET /api/auth/login-method': () => {
          calls += 1
          return calls === 1
            ? json(500, { error: 'server.internal', params: {} })
            : json(200, { has_password: true, oidc: 'none' })
        },
      },
      { path: '/settings/password' },
    )
    await user.click(await screen.findByRole('button', { name: '다시 시도' }))
    expect(await screen.findByLabelText('현재 비밀번호')).toBeInTheDocument()
    expect(calls).toBe(2)
  })

  it('timezone_list_falls_back_without_intl_support', async () => {
    const spy = vi.spyOn(Intl, 'supportedValuesOf').mockImplementation(() => {
      throw new RangeError('unsupported')
    })
    renderPage({}, { role: 'owner', path: '/settings/general' })
    const select = await screen.findByLabelText('시간대')
    expect(within(select).getAllByRole('option')).toHaveLength(1)
    spy.mockRestore()
  })

  it('locale_change_applies', async () => {
    const bodies: unknown[] = []
    const { user } = renderPage(
      {
        'PATCH /api/users/me': async (req) => {
          bodies.push(await req.json())
          return json(200, {
            user: { ...userFixture('guardian'), locale: 'en' },
          })
        },
      },
      { path: '/settings/appearance' },
    )
    const group = await screen.findByRole('radiogroup', { name: '언어' })
    expect(within(group).getByRole('radio', { name: '한국어' })).toBeChecked()
    await user.click(within(group).getByRole('radio', { name: 'English' }))
    expect(
      await screen.findByRole('heading', {
        level: 1,
        name: 'App Display & language',
      }),
    ).toBeInTheDocument()
    expect(bodies).toEqual([{ locale: 'en' }])
  })

  it('theme_change_applies', async () => {
    const { user } = renderPage({}, { path: '/settings/appearance' })
    const group = await screen.findByRole('radiogroup', { name: '테마' })
    expect(
      within(group)
        .getAllByRole('radio')
        .map((r) => r.closest('label')?.textContent),
    ).toEqual(['시스템', '밝게', '어둡게'])
    await user.click(within(group).getByRole('radio', { name: '어둡게' }))
    expect(within(group).getByRole('radio', { name: '어둡게' })).toBeChecked()
    expect(document.documentElement.classList.contains('dark')).toBe(true)
    // 같은 이름 라디오. 화살표 키로 이동.
    await user.keyboard('{ArrowLeft}')
    expect(within(group).getByRole('radio', { name: '밝게' })).toBeChecked()
    expect(document.documentElement.classList.contains('dark')).toBe(false)
  })

  it('profile_save_refreshes_member_names', async () => {
    const { user, queryClient } = renderPage({
      'PATCH /api/users/me': () =>
        json(200, {
          user: { ...userFixture('guardian'), display_name: '엄마' },
        }),
    })
    // 피드 올린 사람 이름용 멤버 목록 캐시.
    queryClient.setQueryData(['users'], { users: [userFixture('guardian')] })
    const name = await screen.findByLabelText('표시 이름')
    await user.clear(name)
    await user.type(name, '엄마')
    await user.click(screen.getByRole('button', { name: '저장' }))
    await waitFor(() => {
      expect(queryClient.getQueryState(['users'])?.isInvalidated).toBe(true)
    })
  })

  it('general_title_has_section', async () => {
    renderPage({}, { role: 'owner', path: '/settings/general' })
    const heading = await screen.findByRole('heading', {
      level: 1,
      name: '시스템 일반',
    })
    expect(
      within(heading).getByText('›').closest('[aria-hidden]'),
    ).not.toBeNull()
  })

  it('general_fields', async () => {
    renderPage({}, { role: 'owner', path: '/settings/general' })
    expect(await screen.findByLabelText('우리 집 이름')).toBeInTheDocument()
    expect(screen.getByLabelText('시간대')).toBeInTheDocument()
    expect(screen.getByLabelText('새 멤버 기본 언어')).toBeInTheDocument()
    expect(screen.queryByLabelText('AVIF 파생물도 만들기')).toBeNull()
    expect(screen.queryByLabelText('휴지통 보관 일수')).toBeNull()
  })

  it('media_saves_changed_fields_only', async () => {
    const bodies: unknown[] = []
    const { user } = renderPage(
      {
        'PATCH /api/settings': async (req) => {
          bodies.push(await req.json())
          return json(200, { ...settingsFixture, derivatives_avif: true })
        },
        'GET /api/derivatives/regenerate': () =>
          json(200, { remaining: 0, failed: 0 }),
      },
      { role: 'owner', path: '/settings/media' },
    )
    expect(
      await screen.findByRole('heading', {
        level: 1,
        name: '시스템 사진 처리',
      }),
    ).toBeInTheDocument()
    await user.click(await screen.findByLabelText('AVIF 파생물도 만들기'))
    await user.click(screen.getByRole('button', { name: '저장' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ derivatives_avif: true }])
    })
    expect(await screen.findByText('설정을 저장했어요')).toBeInTheDocument()
  })

  it('media_trash_retention_patch', async () => {
    const bodies: unknown[] = []
    const { user } = renderPage(
      {
        'PATCH /api/settings': async (req) => {
          bodies.push(await req.json())
          return json(200, { ...settingsFixture, trash_retention_days: 30 })
        },
        'GET /api/derivatives/regenerate': () =>
          json(200, { remaining: 0, failed: 0 }),
      },
      {
        role: 'owner',
        path: '/settings/media',
        settings: { ...settingsFixture, trash_retention_days: 14 },
      },
    )
    const input = await screen.findByLabelText('휴지통 보관 일수')
    await user.clear(input)
    await user.type(input, '30')
    await user.click(screen.getByRole('button', { name: '저장' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ trash_retention_days: 30 }])
    })
  })

  it('primary_baby_picker', async () => {
    const SKY = babyFixture()
    const SEA = babyFixture({
      id: '00000000-0000-4000-8000-0000000000b2',
      name: '바다',
      sort_order: 1,
    })
    // 아기 1명: 고르기 없음.
    const one = renderPage(
      { 'GET /api/babies': () => json(200, [SKY]) },
      { role: 'owner', path: '/settings/general' },
    )
    await screen.findByLabelText('우리 집 이름')
    await waitFor(() => {
      expect(one.queryClient.getQueryData(['babies'])).toEqual([SKY])
    })
    expect(screen.queryByLabelText('대표 아기')).toBeNull()
    cleanup()

    // 2명: 설정 없으면 첫째가 선택된 상태. 바꾸면 그 필드만 PATCH, 뒤에 설정 다시 조회.
    const bodies: unknown[] = []
    let current: Settings = settingsFixture
    let gets = 0
    const { user, queryClient } = renderPage(
      {
        'GET /api/babies': () => json(200, [SKY, SEA]),
        'GET /api/settings': () => {
          gets += 1
          return json(200, current)
        },
        'PATCH /api/settings': async (req) => {
          bodies.push(await req.json())
          current = { ...settingsFixture, primary_baby_id: SEA.id }
          return json(200, current)
        },
      },
      { role: 'owner', path: '/settings/general' },
    )
    const picker = await screen.findByLabelText('대표 아기')
    expect(picker).toHaveValue(SKY.id)
    const getsBefore = gets
    expect(picker).toHaveAccessibleDescription('홈과 달력의 일수 기준이에요.')
    await user.selectOptions(picker, SEA.id)
    await user.click(screen.getByRole('button', { name: '저장' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ primary_baby_id: SEA.id }])
    })
    await waitFor(() => {
      expect(gets).toBe(getsBefore + 1)
    })
    expect(
      queryClient.getQueryData<Settings>(['settings'])?.primary_baby_id,
    ).toBe(SEA.id)
  })

  it('primary_baby_unchanged_not_sent', async () => {
    const SKY = babyFixture()
    const SEA = babyFixture({
      id: '00000000-0000-4000-8000-0000000000b2',
      name: '바다',
      sort_order: 1,
    })
    const bodies: unknown[] = []
    const { user } = renderPage(
      {
        'GET /api/babies': () => json(200, [SKY, SEA]),
        'PATCH /api/settings': async (req) => {
          bodies.push(await req.json())
          return json(200, { ...settingsFixture, instance_name: '새 이름' })
        },
      },
      { role: 'owner', path: '/settings/general' },
    )
    await screen.findByLabelText('대표 아기')
    const input = screen.getByLabelText('우리 집 이름')
    await user.clear(input)
    await user.type(input, '새 이름')
    await user.click(screen.getByRole('button', { name: '저장' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ instance_name: '새 이름' }])
    })
  })

  it('same_device_sessions_have_distinct_labels', async () => {
    const A = session({ id: 'a', last_seen_at: '2026-01-02T00:00:00Z' })
    const B = session({ id: 'b', last_seen_at: '2026-01-03T00:00:00Z' })
    renderPage(
      {
        'GET /api/auth/sessions': () =>
          json(200, { sessions: [CURRENT, A, B] }),
      },
      { path: '/settings/devices' },
    )
    const buttons = await screen.findAllByRole('button', {
      name: /^Firefox 로그아웃/,
    })
    await waitFor(() => {
      const names = screen
        .getAllByRole('button', { name: /^Firefox 로그아웃/ })
        .map((b) => b.getAttribute('aria-label'))
      expect(new Set(names).size).toBe(2)
    })
    expect(buttons).toHaveLength(2)
  })

  it('revoke_other_session', async () => {
    let deleted = ''
    const { user } = renderPage(
      {
        [`DELETE /api/auth/sessions/${OTHER.id}`]: () => {
          deleted = OTHER.id
          return json(204, null)
        },
      },
      { path: '/settings/devices' },
    )
    await screen.findByText('Chrome')
    expect(
      screen.queryByRole('button', { name: /^Chrome 로그아웃/ }),
    ).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /^Firefox 로그아웃/ }))
    await waitFor(() => {
      expect(deleted).toBe(OTHER.id)
    })
  })
})

// 폴링 간격(3초)을 실제로 기다리지 않음. 부하 시 시간 초과 방지.
// 시작 직후 남은 수 표시 → 다음 조회 2개 → 그다음 조회 0개, 간격 두 번.
async function pollUntilDone() {
  await act(() => vi.advanceTimersByTimeAsync(REGEN_POLL_MS))
  await act(() => vi.advanceTimersByTimeAsync(REGEN_POLL_MS))
}

describe('media page regen', () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('regen_flow_posts_and_polls_until_done', async () => {
    let posted: unknown = null
    let polls = 0
    const { user } = renderPage(
      {
        'GET /api/derivatives/regenerate': () => {
          polls += 1
          return json(200, {
            remaining: polls === 1 ? 0 : polls === 2 ? 2 : 0,
            failed: 0,
          })
        },
        'POST /api/derivatives/regenerate': async (req) => {
          posted = await req.json()
          return json(200, { queued: 2 })
        },
      },
      { role: 'owner', path: '/settings/media' },
    )
    await user.click(await screen.findByRole('button', { name: '다시 만들기' }))
    const dialog = await screen.findByRole('alertdialog')
    await user.click(within(dialog).getByRole('button', { name: '없는 것만' }))
    expect(
      await screen.findByText('남은 항목 2개를 만드는 중이에요.'),
    ).toBeInTheDocument()
    expect(posted).toEqual({ missing_only: true })
    await pollUntilDone()
    expect(await screen.findByText('모두 만들었어요.')).toBeInTheDocument()
  })

  it('regen_shows_failed_count_when_done', async () => {
    let polls = 0
    const { user } = renderPage(
      {
        'GET /api/derivatives/regenerate': () => {
          polls += 1
          return json(200, {
            remaining: polls === 1 ? 0 : polls === 2 ? 2 : 0,
            failed: 1,
          })
        },
        'POST /api/derivatives/regenerate': () => json(200, { queued: 2 }),
      },
      { role: 'owner', path: '/settings/media' },
    )
    await user.click(await screen.findByRole('button', { name: '다시 만들기' }))
    const dialog = await screen.findByRole('alertdialog')
    await user.click(within(dialog).getByRole('button', { name: '없는 것만' }))
    expect(
      await screen.findByText('남은 항목 2개를 만드는 중이에요.'),
    ).toBeInTheDocument()
    await pollUntilDone()
    expect(
      await screen.findByText('1개는 만들지 못했어요.'),
    ).toBeInTheDocument()
    expect(screen.queryByText('모두 만들었어요.')).toBeNull()
  })

  it('regen_all_sends_missing_only_false_and_cancel_closes', async () => {
    let posted: unknown = null
    const { user } = renderPage(
      {
        'GET /api/derivatives/regenerate': () =>
          json(200, { remaining: 0, failed: 0 }),
        'POST /api/derivatives/regenerate': async (req) => {
          posted = await req.json()
          return json(200, { queued: 0 })
        },
      },
      { role: 'owner', path: '/settings/media' },
    )
    await user.click(await screen.findByRole('button', { name: '다시 만들기' }))
    await user.click(await screen.findByRole('button', { name: '취소' }))
    await waitFor(() => {
      expect(screen.queryByRole('alertdialog')).toBeNull()
    })
    await user.click(screen.getByRole('button', { name: '다시 만들기' }))
    await user.click(await screen.findByRole('button', { name: '전부' }))
    expect(await screen.findByText('모두 만들었어요.')).toBeInTheDocument()
    expect(posted).toEqual({ missing_only: false })
  })
})
