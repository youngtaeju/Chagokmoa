import { cleanup, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { Role } from '@/api/types'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import { emptyFeedRoutes, settingsFixture, userFixture } from '@/test/fixtures'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
})

afterEach(cleanup)

function renderMe(role: Role, path = '/settings') {
  mockFetch({
    'GET /api/auth/me': () => json(200, { user: userFixture(role) }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/auth/status': () => json(200, { signup_open: false }),
    'GET /api/auth/sessions': () => json(200, { sessions: [] }),
    'POST /api/auth/logout': () => json(204, null),
    ...emptyFeedRoutes,
  })
  return renderApp(path)
}

describe('SettingsPage', () => {
  // 내 계정, 앱, 우리 집(가족, 휴지통), 시스템(owner). 즐겨찾기는 앨범 탭, 아기는 아기 탭으로만.
  it('groups_by_role', async () => {
    const account = [
      ko.me.account,
      [ko.me.profile, ko.me.password, ko.me.devices],
    ]
    const app = [ko.me.app, [ko.me.notifications, ko.me.appearance]]
    const expected = {
      family: [account, app, [ko.me.home, [ko.me.family]]],
      guardian: [account, app, [ko.me.home, [ko.me.family, ko.me.trash]]],
      owner: [
        account,
        app,
        [ko.me.home, [ko.me.family, ko.me.trash]],
        [ko.me.system, [ko.me.general, ko.me.media, ko.me.backup]],
      ],
    } as const
    for (const role of ['family', 'guardian', 'owner'] as const) {
      renderMe(role)
      await screen.findByRole('heading', { name: ko.me.account })
      const groups = screen.getAllByRole('heading', { level: 2 }).map((h) => {
        const section = h.closest('section')
        if (!section) throw new Error('그룹 없음')
        return [
          h.textContent,
          within(section)
            .getAllByRole('link')
            .map((l) => l.textContent),
        ]
      })
      expect(groups).toEqual(expected[role])
      expect(
        screen.queryByRole('link', { name: ko.albums.favorites }),
      ).toBeNull()
      cleanup()
    }

    renderMe('family')
    const home = (
      await screen.findByRole('heading', { name: ko.me.home })
    ).closest('section')
    if (!home) throw new Error('우리 집 그룹 없음')
    expect(
      within(home).getByRole('link', { name: ko.me.family }),
    ).toHaveAttribute('href', '/family')
  })

  it('me_groups_divided_by_line', async () => {
    renderMe('owner')
    const heading = await screen.findByRole('heading', { name: ko.me.home })
    const section = heading.closest('section')
    if (!section) throw new Error('그룹 없음')
    // 그룹 사이 실선, 그룹 첫 줄 진한 라벨, 항목 사이 선 없음, 바탕 한 색.
    expect(section).toHaveClass('border-t', 'border-border')
    expect(heading).toHaveClass('text-foreground', 'font-bold')
    const list = section.querySelector('ul')
    if (!list) throw new Error('목록 없음')
    expect(list).not.toHaveClass('divide-y')
    expect(list).not.toHaveClass('bg-card')
    expect(section.parentElement).not.toHaveClass('bg-muted')
    expect(screen.getByRole('link', { name: /@/ })).not.toHaveClass('bg-card')
    const logout = screen.getByRole('button', { name: '로그아웃' })
    expect(logout).not.toHaveClass('bg-card')
    expect(logout.closest('section')).toHaveClass('border-t')
  })

  it('shows_name_username_role', async () => {
    renderMe('owner')
    const me = userFixture('owner')
    expect(
      await screen.findByRole('heading', { name: me.display_name }),
    ).toBeInTheDocument()
    expect(screen.getByText(`@${me.username}`)).toBeInTheDocument()
    expect(screen.getByText('관리자')).toBeInTheDocument()
  })

  it('logout_from_me', async () => {
    const { user, router } = renderMe('guardian')
    await user.click(await screen.findByRole('button', { name: '로그아웃' }))
    await waitFor(() => {
      expect(router.state.location.href).toBe('/login')
    })
  })

  it('app_version_shown', async () => {
    renderMe('family')
    expect(await screen.findByText(/^앱 버전 \d+\.\d+\.\d+/)).toBeVisible()
  })

  it('subpage_back_without_history', async () => {
    const { user, router } = renderMe('family', '/settings/devices')
    await user.click(await screen.findByRole('button', { name: '뒤로' }))
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/settings')
    })
  })

  it('back_pops_history', async () => {
    const { user, router } = renderMe('family')
    await user.click(await screen.findByRole('link', { name: '로그인한 기기' }))
    await screen.findByRole('heading', { name: '로그인한 기기', level: 1 })
    const length = window.history.length
    await user.click(screen.getByRole('button', { name: '뒤로' }))
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/settings')
    })
    expect(window.history.length).toBe(length)
  })

  it.each(['/settings/backup', '/settings/general', '/settings/media'])(
    'system_pages_owner_only %s',
    async (path) => {
      const { router } = renderMe('guardian', path)
      await waitFor(() => {
        expect(router.state.location.pathname).toBe('/settings')
      })
    },
  )
})
