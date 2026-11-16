import type { ReactNode } from 'react'
import { QueryClientContext } from '@tanstack/react-query'
import { useContext } from 'react'
import { useTranslation } from 'react-i18next'
import { meQueryKey } from '@/auth/session'
import { rememberedInstanceName } from '@/settings/useSettings'
import IconHome from '~icons/material-symbols/home-outline-rounded'
import { AuthBackdrop, AuthHills } from './AuthBackdrop'
import { AuthTopBar } from './AuthTopBar'

// 셸 없는 화면(로그인, 가입, 초대, 없는 경로, 오류, 오프라인) 공용.
// 휴대폰 전체 화면, md 이상 장식 배경 위 가운데 440px 카드.
// 제목 구조: title 있으면 title 이 h1, intro 만 있으면(로그인) 브랜드 앱 이름이 h1.
// 둘 다 없으면 브랜드는 제목 아님(가입, 초대 단계는 내용 안 StepHeading 이 h1).
export function AuthLayout({
  title,
  description,
  intro,
  children,
}: {
  title?: string
  description?: ReactNode
  intro?: ReactNode
  children?: ReactNode
}) {
  const { t } = useTranslation()
  // 조회 없이 캐시만 확인(프로바이더 밖 렌더 허용).
  const signedIn =
    useContext(QueryClientContext)?.getQueryData(meQueryKey) != null
  const home = rememberedInstanceName()
  const Brand = intro && !title ? 'h1' : 'p'
  return (
    <div className="relative isolate flex min-h-dvh flex-col bg-auth-sky md:items-center md:justify-center md:px-8 md:py-24">
      <AuthBackdrop />
      <AuthTopBar
        signedIn={signedIn}
        className="relative mr-6 mt-4 self-end md:absolute md:right-8 md:top-6 md:m-0"
      />
      <main className="relative w-full max-w-md self-center px-6 pb-8 pt-16 md:w-[440px] md:max-w-none md:rounded-[28px] md:border md:border-auth-card-border md:bg-auth-card md:px-10 md:pb-9 md:pt-11 md:shadow-auth-card">
        <div className="flex flex-col items-center gap-3 md:gap-3.5">
          <div className="relative flex items-center gap-2.5 md:gap-3">
            {/* 아이콘 왼쪽 위 반짝 표시. */}
            <svg
              aria-hidden="true"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.2"
              strokeLinecap="round"
              className="absolute -left-5 -top-3.5 size-[22px] text-primary md:-left-[22px] md:-top-4 md:size-[26px]"
            >
              <path d="M14 4 L15.5 9" />
              <path d="M6 8 L10 11" />
              <path d="M3 16 L8 16" />
            </svg>
            <img src="/icon.svg" alt="" className="size-12 md:size-14" />
            <Brand className="text-[28px] font-bold leading-[1.2] tracking-[-0.02em] text-auth-brand md:text-[32px]">
              {t('app.name')}
            </Brand>
          </div>
          {home && (
            <p className="inline-flex items-center gap-1.5 rounded-full bg-accent-soft py-1 pl-2.5 pr-3 text-[13px] font-bold text-accent-ink">
              <IconHome className="size-4" />
              {home}
            </p>
          )}
          {intro && (
            <p className="text-center text-[16px] leading-[1.6] text-auth-intro md:text-[17px]">
              {intro}
            </p>
          )}
        </div>
        <div className="mt-7 flex flex-col gap-6">
          {(title || description) && (
            <div className="grid gap-1">
              {title && <h1 className="text-xl font-bold">{title}</h1>}
              {description && (
                <div className="text-sm text-muted-foreground">
                  {description}
                </div>
              )}
            </div>
          )}
          {children}
        </div>
      </main>
      <AuthHills />
    </div>
  )
}
