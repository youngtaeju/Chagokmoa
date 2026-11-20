import { cleanup, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Invite, Role, User } from '@/api/types'
import i18n from 'i18next'
import { initI18n } from '@/i18n'
import { json, mockFetch } from '@/test/fetch'
import { emptyFeedRoutes, settingsFixture, userFixture } from '@/test/fixtures'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
})

afterEach(() => {
  cleanup()
  Reflect.deleteProperty(navigator, 'share')
  vi.restoreAllMocks()
})

function member(over: Partial<User>): User {
  return { ...userFixture('family'), email: 'a@x.test', ...over }
}

const OWNER = userFixture('owner')
const DAD = member({
  id: '00000000-0000-4000-8000-000000000002',
  username: 'dad',
  display_name: '아빠',
  role: 'guardian',
})
const OTHER_OWNER = member({
  id: '00000000-0000-4000-8000-000000000003',
  username: 'mom2',
  display_name: '엄마2',
  role: 'owner',
})

function invite(over: Partial<Invite> = {}): Invite {
  return {
    id: '00000000-0000-4000-8000-0000000000c1',
    role: 'family',
    status: 'active',
    created_at: '2026-01-01T00:00:00Z',
    created_by: OWNER.id,
    expires_at: '2026-01-04T00:00:00Z',
    revoked_at: null,
    used_at: null,
    used_by: null,
    ...over,
  }
}

function renderMembers(
  extra: Parameters<typeof mockFetch>[0] = {},
  {
    role = 'owner',
    users = [OWNER, DAD],
  }: { role?: Role; users?: User[] } = {},
) {
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => json(200, { user: userFixture(role) }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/users': () => json(200, { users }),
    'GET /api/invites': () => json(200, { invites: [] }),
    'GET /api/auth/status': () =>
      json(200, { signup_open: false, mail_enabled: false }),
    ...extra,
  })
  return renderApp('/family')
}

const mailOn = {
  'GET /api/auth/status': () =>
    json(200, { signup_open: false, mail_enabled: true }),
}

describe('FamilyPage', () => {
  it('members_visible_to_family', async () => {
    renderMembers({}, { role: 'family', users: [OWNER, DAD] })
    expect(await screen.findByText('아빠')).toBeInTheDocument()
    const dadRow = screen.getByText('아빠').closest('li')
    expect(dadRow).not.toBeNull()
    if (dadRow) expect(within(dadRow).getByText('보호자')).toBeInTheDocument()
    const selfRow = screen.getByText('owner').closest('li')
    expect(selfRow).not.toBeNull()
    if (selfRow) expect(within(selfRow).getByText('나')).toBeInTheDocument()
    expect(screen.queryByText('a@x.test')).not.toBeInTheDocument()
  })

  it('invite_controls_owner_only', async () => {
    renderMembers({}, { role: 'guardian', users: [OWNER, DAD] })
    expect(await screen.findByText('아빠')).toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: '가족 초대하기' }),
    ).not.toBeInTheDocument()
    expect(
      screen.queryByRole('heading', { name: '대기 중인 초대' }),
    ).not.toBeInTheDocument()
    // 행을 눌러도 시트 없음.
    expect(
      screen.queryByRole('button', { name: /아빠/ }),
    ).not.toBeInTheDocument()
  })

  // 설정 탭 하위 화면. 직접 진입이면 뒤로 = 설정 화면, 초대 버튼은 머리글 오른쪽.
  it('header_back_and_invite', async () => {
    const { user, router } = renderMembers({}, { users: [OWNER, DAD] })
    const header = (
      await screen.findByRole('heading', { level: 1, name: '가족' })
    ).closest('header')
    if (!header) throw new Error('머리글 없음')
    expect(
      within(header).getByRole('button', { name: '가족 초대하기' }),
    ).toBeInTheDocument()
    await user.click(within(header).getByRole('button', { name: '뒤로' }))
    await waitFor(() => {
      expect(router.state.location.pathname).toBe('/settings')
    })
  })

  it('role_labels_ko_en', async () => {
    expect(i18n.t('roles.owner')).toBe('관리자')
    expect(i18n.t('roles.guardian')).toBe('보호자')
    expect(i18n.t('roles.family')).toBe('가족')
    await i18n.changeLanguage('en')
    expect(i18n.t('roles.owner')).toBe('Admin')
    expect(i18n.t('roles.guardian')).toBe('Guardian')
    expect(i18n.t('roles.family')).toBe('Family')
    expect(i18n.t('roles.familyDesc')).not.toBe('roles.familyDesc')
    await i18n.changeLanguage('ko')
  })

  async function createInvite(
    user: ReturnType<typeof renderMembers>['user'],
    bodies: unknown[] = [],
  ) {
    await user.click(
      await screen.findByRole('button', { name: '가족 초대하기' }),
    )
    const sheet = await screen.findByRole('dialog')
    await user.click(
      within(sheet).getByRole('button', { name: '초대 링크 만들기' }),
    )
    await screen.findByDisplayValue('http://x/invite/r')
    return bodies
  }

  const createRoute = (bodies: unknown[], mailSent: boolean | null = null) => ({
    'POST /api/invites': async (req: Request) => {
      bodies.push(await req.json())
      return json(201, {
        url: 'http://x/invite/r',
        invite: invite(),
        mail_sent: mailSent,
      })
    },
  })

  async function openInvite(user: ReturnType<typeof renderMembers>['user']) {
    await user.click(
      await screen.findByRole('button', { name: '가족 초대하기' }),
    )
    return screen.findByRole('dialog')
  }

  it('email_field_only_when_mail_enabled', async () => {
    const off: unknown[] = []
    const first = renderMembers(createRoute(off))
    let sheet = await openInvite(first.user)
    await waitFor(() => {
      expect(first.queryClient.getQueryData(['auth', 'status'])).toBeDefined()
    })
    expect(within(sheet).queryByLabelText('이메일로도 보내기(선택)')).toBeNull()
    await first.user.click(
      within(sheet).getByRole('button', { name: '초대 링크 만들기' }),
    )
    await screen.findByDisplayValue('http://x/invite/r')
    expect(off).toEqual([{ role: 'family', expires_in_hours: 72 }])
    cleanup()

    const on: unknown[] = []
    const { user } = renderMembers({ ...mailOn, ...createRoute(on, true) })
    sheet = await openInvite(user)
    await user.type(
      await within(sheet).findByLabelText('이메일로도 보내기(선택)'),
      'dad@x.test',
    )
    await user.click(
      within(sheet).getByRole('button', { name: '초대 링크 만들기' }),
    )
    await screen.findByDisplayValue('http://x/invite/r')
    expect(on).toEqual([
      { role: 'family', expires_in_hours: 72, email: 'dad@x.test' },
    ])
  })

  it('invite_mail_result_line', async () => {
    for (const sent of [true, false]) {
      const { user } = renderMembers({ ...mailOn, ...createRoute([], sent) })
      const sheet = await openInvite(user)
      await user.type(
        await within(sheet).findByLabelText('이메일로도 보내기(선택)'),
        'dad@x.test',
      )
      await user.click(
        within(sheet).getByRole('button', { name: '초대 링크 만들기' }),
      )
      expect(
        await screen.findByDisplayValue('http://x/invite/r'),
      ).toBeInTheDocument()
      expect(
        screen.getByText(
          sent
            ? 'dad@x.test로 보냈어요.'
            : '메일을 보내지 못했어요. 링크를 직접 공유해 주세요.',
        ),
      ).toBeInTheDocument()
      cleanup()
    }
  })

  it('share_used_when_available', async () => {
    const share = vi.fn(() => Promise.resolve())
    Object.defineProperty(navigator, 'share', {
      value: share,
      configurable: true,
    })
    const bodies: unknown[] = []
    const { user } = renderMembers(createRoute(bodies))
    await createInvite(user)
    expect(bodies).toEqual([{ role: 'family', expires_in_hours: 72 }])
    // 만든 직후 자동 공유 없음. 버튼 클릭에서만.
    expect(share).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: '공유하기' }))
    expect(share).toHaveBeenCalledWith(
      expect.objectContaining({ url: 'http://x/invite/r' }),
    )
    expect(screen.queryByText('링크를 복사했어요')).not.toBeInTheDocument()
  })

  it('share_abort_does_nothing', async () => {
    const share = vi.fn(() =>
      Promise.reject(new DOMException('cancel', 'AbortError')),
    )
    Object.defineProperty(navigator, 'share', {
      value: share,
      configurable: true,
    })
    const writeText = vi.fn(() => Promise.resolve())
    vi.spyOn(navigator.clipboard, 'writeText').mockImplementation(writeText)
    const { user } = renderMembers(createRoute([]))
    await createInvite(user)
    await user.click(screen.getByRole('button', { name: '공유하기' }))
    expect(share).toHaveBeenCalled()
    expect(writeText).not.toHaveBeenCalled()
  })

  it('copy_fallback', async () => {
    Object.defineProperty(navigator, 'share', {
      value: undefined,
      configurable: true,
    })
    const { user } = renderMembers(createRoute([]))
    await createInvite(user)
    expect(screen.queryByRole('button', { name: '공유하기' })).toBeNull()
    await user.click(screen.getByRole('button', { name: '복사' }))
    await waitFor(async () => {
      expect(await navigator.clipboard.readText()).toBe('http://x/invite/r')
    })
    expect(await screen.findByText('복사했어요')).toBeInTheDocument()
    expect(screen.getByDisplayValue('http://x/invite/r')).toHaveAttribute(
      'readonly',
    )
  })

  it('member_sheet_change_role', async () => {
    const bodies: unknown[] = []
    const { user } = renderMembers({
      [`PATCH /api/users/${DAD.id}`]: async (req) => {
        bodies.push(await req.json())
        return json(200, { user: { ...DAD, role: 'family' } })
      },
    })
    await user.click(await screen.findByRole('button', { name: /아빠/ }))
    const sheet = await screen.findByRole('dialog')
    await user.selectOptions(
      within(sheet).getByRole('combobox', { name: '아빠 역할' }),
      'family',
    )
    await waitFor(() => {
      expect(bodies).toEqual([{ role: 'family' }])
    })
  })

  it('suspend_asks_confirmation', async () => {
    const bodies: unknown[] = []
    const { user } = renderMembers({
      [`PATCH /api/users/${DAD.id}`]: async (req) => {
        bodies.push(await req.json())
        return json(200, {
          user: { ...DAD, disabled_at: '2026-01-02T00:00:00Z' },
        })
      },
    })
    await user.click(await screen.findByRole('button', { name: /아빠/ }))
    const sheet = await screen.findByRole('dialog')
    await user.click(within(sheet).getByRole('button', { name: '정지' }))
    const confirm = await screen.findByRole('alertdialog')
    expect(bodies).toEqual([])
    await user.click(within(confirm).getByRole('button', { name: '정지' }))
    await waitFor(() => {
      expect(bodies).toEqual([{ disabled: true }])
    })
  })

  it('owner_rows_not_editable', async () => {
    renderMembers({}, { users: [OWNER, OTHER_OWNER, DAD] })
    await screen.findByRole('button', { name: /아빠/ })
    expect(screen.getByText('엄마2')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /엄마2/ })).toBeNull()
    expect(screen.queryByRole('button', { name: /owner/ })).toBeNull()
  })

  it('revoke_active_invite_only', async () => {
    let revoked = ''
    const { user } = renderMembers({
      'GET /api/invites': () =>
        json(200, {
          invites: [
            invite(),
            invite({
              id: '00000000-0000-4000-8000-0000000000c2',
              status: 'used',
            }),
          ],
        }),
      'DELETE /api/invites/00000000-0000-4000-8000-0000000000c1': () => {
        revoked = 'c1'
        return json(204, null)
      },
    })
    // 사용됨 초대는 목록에서 제외. 중복이면 findByRole 이 실패.
    await user.click(await screen.findByRole('button', { name: /초대 회수/ }))
    expect(screen.getAllByRole('button', { name: /초대 회수/ })).toHaveLength(1)
    expect(screen.queryByText('사용됨')).toBeNull()
    await waitFor(() => {
      expect(revoked).toBe('c1')
    })
  })

  it('revoke_buttons_have_distinct_labels', async () => {
    renderMembers({
      'GET /api/invites': () =>
        json(200, {
          invites: [
            invite(),
            invite({
              id: '00000000-0000-4000-8000-0000000000c2',
              expires_at: '2026-01-05T00:00:00Z',
            }),
          ],
        }),
    })
    const buttons = await screen.findAllByRole('button', { name: /초대 회수/ })
    const names = buttons.map((b) => b.getAttribute('aria-label'))
    expect(names).toHaveLength(2)
    expect(new Set(names).size).toBe(2)
    expect(names[0]).toContain('1월 4일')
  })
})
