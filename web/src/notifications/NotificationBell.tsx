import { Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import IconBell from '~icons/material-symbols/notifications-outline-rounded'
import { formatBadge, useUnreadCount } from './useUnreadCount'

// 홈 머리글 종. 터치 영역 44px, 안 읽은 수 배지.
export function NotificationBell() {
  const { t } = useTranslation()
  const count = useUnreadCount() ?? 0
  const badge = formatBadge(count)
  return (
    <Link
      // 알림 라우트는 알림 화면 추가 때 생성. 그때 이 주석과 함께 제거.
      // @ts-expect-error 알림 라우트 아직 없음
      to="/notifications"
      aria-label={
        badge
          ? t('notifications.bellNew', { count: badge })
          : t('notifications.title')
      }
      className="relative flex size-11 shrink-0 items-center justify-center rounded-full"
    >
      <IconBell aria-hidden className="size-6" />
      {badge && (
        <span
          aria-hidden
          className="absolute top-1 right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-destructive px-1 text-2xs font-semibold text-white tabular-nums"
        >
          {badge}
        </span>
      )}
    </Link>
  )
}
