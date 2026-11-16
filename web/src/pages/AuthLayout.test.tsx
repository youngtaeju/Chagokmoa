import { QueryClientProvider } from '@tanstack/react-query'
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterAll, afterEach, beforeEach, describe, expect, it } from 'vitest'
import { meQueryKey } from '@/auth/session'
import { initI18n } from '@/i18n'
import { userFixture } from '@/test/fixtures'
import { createTestQueryClient } from '@/test/query'
import { setThemePref } from '@/theme/theme'
import { AuthLayout } from './AuthLayout'

type MediaListener = (e: { matches: boolean }) => void

// 시스템 다크 전환 재현용. 테마 모듈은 파일 안에서 1회만 초기화.
let systemDark = false
const mediaListeners = new Set<MediaListener>()
const originalMatchMedia = window.matchMedia.bind(window)
window.matchMedia = (query: string): MediaQueryList =>
  ({
    get matches() {
      return query === '(prefers-color-scheme: dark)' && systemDark
    },
    media: query,
    addEventListener: (_: string, l: MediaListener) => mediaListeners.add(l),
    removeEventListener: (_: string, l: MediaListener) =>
      mediaListeners.delete(l),
  }) as unknown as MediaQueryList

function setSystemDark(dark: boolean) {
  systemDark = dark
  act(() => {
    mediaListeners.forEach((l) => {
      l({ matches: dark })
    })
  })
}

beforeEach(() => {
  initI18n('ko')
  localStorage.clear()
})

afterEach(() => {
  setSystemDark(false)
  setThemePref('system')
  localStorage.clear()
  document.documentElement.classList.remove('dark')
})

afterAll(() => {
  window.matchMedia = originalMatchMedia
})

describe('AuthLayout', () => {
  it('brand_is_h1_with_intro', () => {
    render(<AuthLayout intro="소개 문구" />)
    expect(
      screen.getByRole('heading', { level: 1, name: '차곡모아' }),
    ).toBeInTheDocument()
    expect(screen.getByText('소개 문구')).toBeInTheDocument()
  })

  // 가입, 초대 단계는 내용 안 StepHeading 이 h1.
  it('brand_is_not_heading_without_title_or_intro', () => {
    render(<AuthLayout />)
    expect(screen.getByText('차곡모아')).toBeInTheDocument()
    expect(screen.queryByRole('heading')).not.toBeInTheDocument()
  })

  it('title_is_h1_when_given', () => {
    render(<AuthLayout title="없는 페이지" description="설명" />)
    expect(
      screen.getByRole('heading', { level: 1, name: '없는 페이지' }),
    ).toBeInTheDocument()
    expect(screen.getByText('차곡모아')).toBeInTheDocument()
    expect(
      screen.queryByRole('heading', { name: '차곡모아' }),
    ).not.toBeInTheDocument()
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1)
  })

  it('home_pill_only_when_remembered', () => {
    const { unmount } = render(<AuthLayout />)
    expect(screen.queryByText('하늘이네')).not.toBeInTheDocument()
    unmount()

    localStorage.setItem('cgm.instance.name', '하늘이네')
    render(<AuthLayout />)
    expect(screen.getByText('하늘이네')).toBeInTheDocument()
  })

  it('theme_button_flips_system_dark_to_light', async () => {
    setSystemDark(true)
    render(<AuthLayout />)
    expect(document.documentElement).toHaveClass('dark')

    await userEvent.click(
      screen.getByRole('button', { name: '라이트 모드로 바꾸기' }),
    )

    expect(localStorage.getItem('cgm.theme')).toBe('light')
    expect(document.documentElement).not.toHaveClass('dark')
    expect(
      screen.getByRole('button', { name: '다크 모드로 바꾸기' }),
    ).toBeInTheDocument()
  })

  it('theme_button_flips_system_light_to_dark', async () => {
    render(<AuthLayout />)
    expect(document.documentElement).not.toHaveClass('dark')

    await userEvent.click(
      screen.getByRole('button', { name: '다크 모드로 바꾸기' }),
    )

    expect(localStorage.getItem('cgm.theme')).toBe('dark')
    expect(document.documentElement).toHaveClass('dark')
  })

  // 앱 셸 안 렌더 시 사이드바를 덮지 않도록 viewport 고정 금지.
  it('backdrop_not_viewport_fixed', () => {
    const { container } = render(<AuthLayout />)
    expect(container.querySelector('.fixed')).toBeNull()
  })

  it('signed_in_hides_language_keeps_theme', () => {
    const client = createTestQueryClient()
    client.setQueryData(meQueryKey, userFixture('owner'))
    render(
      <QueryClientProvider client={client}>
        <AuthLayout title="없는 페이지" />
      </QueryClientProvider>,
    )
    expect(
      screen.queryByRole('group', { name: '언어' }),
    ).not.toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: '다크 모드로 바꾸기' }),
    ).toBeInTheDocument()
  })

  it('signed_out_shows_language', () => {
    render(<AuthLayout />)
    const group = screen.getByRole('group', { name: '언어' })
    expect(group).toContainElement(
      screen.getByRole('button', { name: 'English', pressed: false }),
    )
  })

  // 휴대폰 언덕은 스크롤 끝. 본문 뒤 흐름이라 긴 내용과 겹치지 않음.
  it('backdrop_after_content_on_mobile', () => {
    const { container } = render(<AuthLayout title="없는 페이지" />)
    const hills = container.querySelector('[data-auth-hills]')
    const main = screen.getByRole('main')
    expect(hills).not.toBeNull()
    expect(hills).toHaveAttribute('aria-hidden', 'true')
    expect(main).not.toContainElement(hills as HTMLElement)
    expect(
      main.compareDocumentPosition(hills as Node) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
  })
})
