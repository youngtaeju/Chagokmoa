import { useMutation, useQueryClient } from '@tanstack/react-query'
import IconCheck from '~icons/material-symbols/check-rounded'
import { useId, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { api, call } from '@/api/client'
import type { User } from '@/api/types'
import { setSignedIn } from '@/auth/session'
import { FormError } from '@/components/Field'
import { useTheme, type ThemePref } from '@/theme/theme'

// 미리보기 칸 색. 앱 테마와 무관하게 고정(styles.css 라이트, 다크의 background, muted).
const half = (l: string, d: string) =>
  `linear-gradient(90deg, ${l} 50%, ${d} 50%)`
const PREVIEW: Record<ThemePref, { page: string; bar: string }> = {
  light: { page: '#ffffff', bar: '#f4f5f6' },
  dark: { page: '#121416', bar: '#1c1f22' },
  system: { page: half('#ffffff', '#121416'), bar: half('#f4f5f6', '#1c1f22') },
}

const THEMES = ['system', 'light', 'dark'] as const

// 나 화면과 같은 규칙: 그룹 위 실선, 진한 라벨.
function Group({
  title,
  children,
}: {
  title: string
  children: (labelId: string) => ReactNode
}) {
  const id = useId()
  return (
    <section className="border-t border-border pt-3 pb-2">
      <h2 id={id} className="px-4 pb-2 text-[13px] font-bold">
        {title}
      </h2>
      {children(id)}
    </section>
  )
}

// 고르는 즉시 적용. 같은 name 의 기본 라디오라 화살표 키 이동 지원.
function ThemeGroup() {
  const { t } = useTranslation()
  const [theme, setTheme] = useTheme()
  const name = useId()
  const labels: Record<ThemePref, string> = {
    system: t('settings.themeSystem'),
    light: t('settings.themeLight'),
    dark: t('settings.themeDark'),
  }
  return (
    <Group title={t('settings.theme')}>
      {(labelId) => (
        <div
          role="radiogroup"
          aria-labelledby={labelId}
          className="grid grid-cols-3 gap-3 px-4 pb-2"
        >
          {THEMES.map((p) => (
            <label key={p} className="grid cursor-pointer gap-1.5">
              <input
                type="radio"
                name={name}
                value={p}
                checked={theme === p}
                onChange={() => {
                  setTheme(p)
                }}
                className="peer sr-only"
              />
              <span
                aria-hidden
                className="grid h-20 grid-rows-[0.875rem_1fr] overflow-hidden rounded-xl border-2 border-border peer-checked:border-accent-ink peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-ring"
                style={{ background: PREVIEW[p].page }}
              >
                <span style={{ background: PREVIEW[p].bar }} />
              </span>
              <span className="text-center text-xs text-muted-foreground peer-checked:font-bold peer-checked:text-foreground">
                {labels[p]}
              </span>
            </label>
          ))}
        </div>
      )}
    </Group>
  )
}

// 언어는 고르는 즉시 저장.
export function AppearanceSection({ user }: { user: User }) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const name = useId()
  const save = useMutation({
    mutationFn: (locale: string) =>
      call(api.PATCH('/api/users/me', { body: { locale } })),
    // 캐시 갱신과 UI 언어 전환 동시 처리.
    onSuccess: ({ user: updated }) => {
      setSignedIn(qc, updated)
    },
  })
  const locales = [
    { value: 'ko', label: t('settings.localeKo') },
    { value: 'en', label: t('settings.localeEn') },
  ]
  return (
    // 휴대폰은 실선이 화면 끝까지(-mx-4), 넓은 화면은 가운데 열.
    <div className="-mx-4 grid sm:mx-0">
      <ThemeGroup />
      <Group title={t('settings.language')}>
        {(labelId) => (
          <div role="radiogroup" aria-labelledby={labelId}>
            {locales.map((l) => (
              <label
                key={l.value}
                lang={l.value}
                className="flex min-h-14 cursor-pointer items-center gap-3 px-4 text-[15px] has-[:focus-visible]:outline-2 has-[:focus-visible]:-outline-offset-2 has-[:focus-visible]:outline-ring has-[:disabled]:cursor-default has-[:disabled]:opacity-50"
              >
                <input
                  type="radio"
                  name={name}
                  value={l.value}
                  checked={user.locale === l.value}
                  disabled={save.isPending}
                  onChange={() => {
                    save.mutate(l.value)
                  }}
                  className="peer sr-only"
                />
                <span className="flex-1">{l.label}</span>
                <IconCheck
                  aria-hidden
                  className="invisible size-6 text-accent-ink peer-checked:visible"
                />
              </label>
            ))}
          </div>
        )}
      </Group>
      <div className="px-4">
        <FormError error={save.error} />
      </div>
    </div>
  )
}
