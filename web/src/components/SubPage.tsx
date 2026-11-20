import { useCanGoBack, useRouter } from '@tanstack/react-router'
import IconBack from '~icons/material-symbols/arrow-back-rounded'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'

// 설정 탭, 앨범 탭 하위 화면, 알림 화면 공통 틀. 셸 머리글을 대신해 뒤로(기록 있으면 되돌아가기, 직접 진입이면 backTo) + 제목 + 오른쪽 actions.
export function SubPage({
  title,
  section,
  actions,
  backTo = '/settings',
  children,
}: {
  title: string
  // 묶음 이름. `시스템 › 일반` 형태 제목.
  section?: string
  actions?: ReactNode
  backTo?: '/settings' | '/albums' | '/'
  children: ReactNode
}) {
  const { t } = useTranslation()
  const router = useRouter()
  const canGoBack = useCanGoBack()
  return (
    // 열 최소 폭 0. 말줄임 제목(nowrap)의 글자 폭만큼 열이 넓어지지 않게.
    <div className="-mt-4 grid grid-cols-1 gap-4">
      <header className="sticky top-0 z-10 -mx-4 flex min-h-14 items-center gap-1 bg-background px-2 py-1">
        <Button
          variant="ghost"
          size="icon"
          aria-label={t('me.back')}
          onClick={() => {
            if (canGoBack) router.history.back()
            else void router.navigate({ to: backTo, replace: true })
          }}
        >
          <IconBack aria-hidden className="size-6" />
        </Button>
        {section ? (
          <h1 className="min-w-0 flex-1 text-lg font-bold">
            <span className="sr-only">{`${section} ${title}`}</span>
            {/* 좁은 화면은 묶음 이름부터 말줄임. */}
            <span aria-hidden className="flex min-w-0 items-baseline">
              <span className="min-w-0 truncate font-semibold text-muted-foreground">
                {section}
              </span>
              <span className="shrink-0 px-1.5 text-muted-foreground">›</span>
              <span className="min-w-0 shrink-[0.01] truncate">{title}</span>
            </span>
          </h1>
        ) : (
          <h1 className="min-w-0 flex-1 truncate text-lg font-bold">{title}</h1>
        )}
        {actions}
      </header>
      {children}
    </div>
  )
}
