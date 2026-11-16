import { useTranslation } from 'react-i18next'
import { chooseLocale, type Locale } from '@/i18n'
import { cn } from '@/lib/utils'
import { setThemePref, useIsDark } from '@/theme/theme'
import IconMoon from '~icons/material-symbols/dark-mode-outline-rounded'
import IconSun from '~icons/material-symbols/light-mode-outline-rounded'

const LANGUAGES: { lng: Locale; label: string }[] = [
  { lng: 'ko', label: '한국어' },
  { lng: 'en', label: 'English' },
]

// 언어 알약 + 테마 버튼. 로그인 상태면 계정 언어가 우선이라 언어 전환 숨김.
export function AuthTopBar({
  signedIn,
  className,
}: {
  signedIn: boolean
  className?: string
}) {
  const { t, i18n } = useTranslation()
  const dark = useIsDark()
  return (
    <div className={cn('flex items-center gap-1.5', className)}>
      {!signedIn && (
        <div
          role="group"
          aria-label={t('auth.language')}
          className="flex gap-0.5 rounded-full border border-auth-card-border bg-auth-card p-[3px]"
        >
          {LANGUAGES.map(({ lng, label }) => {
            const active = i18n.language === lng
            return (
              <button
                key={lng}
                type="button"
                lang={lng}
                aria-pressed={active}
                onClick={() => {
                  chooseLocale(lng)
                }}
                className={cn(
                  'min-h-[38px] rounded-full px-3 text-[13px]',
                  active
                    ? 'bg-accent-soft font-bold text-accent-ink'
                    : 'font-semibold text-muted-foreground hover:bg-accent',
                )}
              >
                {label}
              </button>
            )
          })}
        </div>
      )}
      {/* 아이콘은 지금 보이는 상태(라이트 해, 다크 달). */}
      <button
        type="button"
        aria-label={dark ? t('auth.themeToLight') : t('auth.themeToDark')}
        onClick={() => {
          setThemePref(dark ? 'light' : 'dark')
        }}
        className="flex size-11 items-center justify-center rounded-full border border-auth-card-border bg-auth-card text-accent-ink hover:bg-accent"
      >
        {dark ? (
          <IconMoon className="size-5" />
        ) : (
          <IconSun className="size-5" />
        )}
      </button>
    </div>
  )
}
