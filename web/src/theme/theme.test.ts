import { afterEach, describe, expect, it, vi } from 'vitest'
import initScript from '../../public/theme-init.js?raw'
import { applyTheme, getThemePref, setThemePref } from './theme'

afterEach(() => {
  document.head.querySelector('meta[data-app-theme]')?.remove()
  document.documentElement.classList.remove('dark')
  vi.restoreAllMocks()
})

describe('theme', () => {
  // 첫 페인트 스크립트와 같은 키. 어긋나면 새로고침 때 테마 깜빡임.
  it('theme_init_uses_same_key', () => {
    expect(initScript).toContain("localStorage.getItem('cgm.theme')")
    setThemePref('dark')
    expect(localStorage.getItem('cgm.theme')).toBe('dark')
    localStorage.clear()
  })

  it('system_follows_media', () => {
    applyTheme('system', true)
    expect(document.documentElement).toHaveClass('dark')
    applyTheme('system', false)
    expect(document.documentElement).not.toHaveClass('dark')
  })

  it('explicit_overrides_system', () => {
    applyTheme('light', true)
    expect(document.documentElement).not.toHaveClass('dark')
    applyTheme('dark', false)
    expect(document.documentElement).toHaveClass('dark')
  })

  it('theme_color_follows_choice', () => {
    applyTheme('light', true)
    const metas = () =>
      document.head.querySelectorAll<HTMLMetaElement>(
        'meta[name="theme-color"]:not([media])',
      )
    expect(metas()).toHaveLength(1)
    expect(metas()[0]?.content).toBe('#FFFFFF')
    applyTheme('system', true)
    expect(metas()).toHaveLength(1)
    expect(metas()[0]?.content).toBe('#121416')
    expect(document.head.querySelector('meta[name="theme-color"]')).toBe(
      metas()[0],
    )
  })

  it('storage_failure_defaults_to_system', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied')
    })
    expect(getThemePref()).toBe('system')
  })
})
