import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { AppNotification } from '@/api/types'
import { useMe } from '@/auth/session'
import { Button } from '@/components/ui/button'
import { errorMessage } from '@/i18n'
import { SubPage } from '@/components/SubPage'
import { useUsers } from '@/members/useUsers'
import { PushBanner } from '@/push/PushBanner'
import { useSettings, useToday } from '@/settings/useSettings'
import { sectionOf, type Section } from './links'
import { NotificationRow } from './NotificationRow'
import { useMarkSeen, useNotifications } from './useNotifications'

const SECTIONS: Section[] = ['today', 'week', 'earlier']

export function NotificationsPage() {
  const { t } = useTranslation()
  const timezone =
    useSettings()?.timezone ?? Intl.DateTimeFormat().resolvedOptions().timeZone
  const today = useToday()
  const meId = useMe().user?.id
  const users = useUsers().data?.users ?? []
  const {
    items,
    seenAt: firstSeenAt,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
    isFetchNextPageError,
    isLoadingError,
    status,
    error,
    refetch,
  } = useNotifications()
  const markSeen = useMarkSeen()

  // 강조 기준은 첫 페이지의 seen_at 고정. seen 호출 뒤 받는 페이지는 더 새로운 값.
  const [seenAt, setSeenAt] = useState<string | null>(null)
  if (seenAt === null && firstSeenAt) setSeenAt(firstSeenAt)
  const posted = useRef(false)
  useEffect(() => {
    if (seenAt === null || posted.current) return
    posted.current = true
    markSeen()
  }, [seenAt, markSeen])

  const sentinel = useRef<HTMLDivElement>(null)
  const autoLoad = hasNextPage && !isFetchingNextPage && !isFetchNextPageError
  useEffect(() => {
    const el = sentinel.current
    if (!el || !autoLoad) return
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) void fetchNextPage()
      },
      { rootMargin: '600px 0px' },
    )
    io.observe(el)
    return () => {
      io.disconnect()
    }
  }, [autoLoad, fetchNextPage])

  const groups = new Map<Section, AppNotification[]>()
  for (const n of items) {
    const s = sectionOf(n.updated_at, today, timezone)
    groups.set(s, [...(groups.get(s) ?? []), n])
  }
  const seenMs = seenAt === null ? null : new Date(seenAt).getTime()
  const now = new Date()

  return (
    <SubPage title={t('notifications.title')} backTo="/">
      <PushBanner />
      {status === 'pending' && (
        <p className="text-sm text-muted-foreground">{t('common.loading')}</p>
      )}
      {isLoadingError && (
        <div className="grid justify-items-start gap-3">
          <p className="text-sm">{errorMessage(t, error)}</p>
          <Button variant="outline" onClick={() => void refetch()}>
            {t('common.retry')}
          </Button>
        </div>
      )}
      {status === 'success' && items.length === 0 && (
        <p className="py-16 text-center text-base">
          {t('notifications.empty')}
        </p>
      )}
      {SECTIONS.map((s) => {
        const list = groups.get(s)
        if (!list) return null
        return (
          <section key={s} className="-mx-2 grid gap-1">
            <h2 className="px-2 text-sm font-semibold text-muted-foreground">
              {t(`notifications.${s}`)}
            </h2>
            <ul className="grid gap-0.5">
              {list.map((n) => (
                <NotificationRow
                  key={n.id}
                  n={n}
                  isNew={
                    seenMs !== null && new Date(n.updated_at).getTime() > seenMs
                  }
                  meId={meId}
                  users={users}
                  now={now}
                  timezone={timezone}
                />
              ))}
            </ul>
          </section>
        )
      })}
      <div ref={sentinel} />
      {isFetchingNextPage && (
        <p className="py-4 text-center text-sm text-muted-foreground">
          {t('common.loading')}
        </p>
      )}
      {isFetchNextPageError && (
        <div className="grid justify-items-center gap-3 py-4">
          <p className="text-sm">{errorMessage(t, error)}</p>
          <Button variant="outline" onClick={() => void fetchNextPage()}>
            {t('common.retry')}
          </Button>
        </div>
      )}
    </SubPage>
  )
}
