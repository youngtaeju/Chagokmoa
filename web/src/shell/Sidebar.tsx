import { Link, useRouterState } from '@tanstack/react-router'
import IconAdd from '~icons/material-symbols/add-rounded'
import IconTrashFill from '~icons/material-symbols/delete-rounded'
import IconBell from '~icons/material-symbols/notifications-outline-rounded'
import IconBellFill from '~icons/material-symbols/notifications-rounded'
import IconTrash from '~icons/material-symbols/delete-outline-rounded'
import { useTranslation } from 'react-i18next'
import { hasRole, useMe } from '@/auth/session'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { formatBadge, useUnreadCount } from '@/notifications/useUnreadCount'
import { isNavActive, useNavItems } from './navItems'

// 보조 메뉴(탭에 없는 목적지). 휴지통은 업로드 권한 있는 사람만, 알림은 모두. 여기 있는 주소에서는 주 메뉴 "나" 선택 표시 안 함.
const MORE = [
  {
    to: '/me/trash',
    label: 'nav.trash',
    icon: IconTrash,
    activeIcon: IconTrashFill,
  },
] as const

const LINK =
  'flex h-11 w-full items-center gap-3 rounded-lg px-3 text-sm text-muted-foreground hover:bg-muted'
const LINK_ACTIVE = 'font-semibold text-accent-ink'

// 1024px 이상 왼쪽 220px. 앱 로고와 이름, 탭과 같은 메뉴, 보조 메뉴(휴지통), 맨 아래 추가하기.
// onAdd: 휴대폰 떠 있는 (+) 와 같은 셸의 추가 시트 열기.
export function Sidebar({ onAdd }: { onAdd: () => void }) {
  const { t } = useTranslation()
  const { user } = useMe()
  const items = useNavItems()
  const canUpload = hasRole(user, 'guardian')
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  // 현재 항목 하나만 선택. 보조 메뉴 주소면 주 메뉴 선택 없음.
  const inNotifications =
    pathname === '/notifications' || pathname.startsWith('/notifications/')
  const inMore =
    inNotifications ||
    (canUpload &&
      MORE.some(({ to }) => pathname === to || pathname.startsWith(`${to}/`)))
  const badge = formatBadge(useUnreadCount() ?? 0)
  return (
    <div className="sticky top-0 hidden h-dvh w-[220px] shrink-0 flex-col gap-4 border-r border-border px-3 py-4 lg:flex">
      <p className="flex items-center gap-2 px-3 text-base font-bold text-auth-brand">
        <img
          src="/icon.svg"
          alt=""
          width={26}
          height={26}
          className="size-[26px] shrink-0 rounded-md"
        />
        {t('app.name')}
      </p>
      <nav aria-label={t('nav.main')}>
        <ul className="grid gap-1">
          {items.map((item) => {
            const active = !inMore && isNavActive(item, pathname)
            const Icon = active ? item.activeIcon : item.icon
            return (
              <li key={item.to}>
                <Link
                  to={item.to}
                  // 선택은 isNavActive 기준. 라우터 자체 판정은 정확히 일치만.
                  activeOptions={{ exact: true, includeSearch: false }}
                  aria-current={active ? 'page' : undefined}
                  className={cn(LINK, active && LINK_ACTIVE)}
                >
                  <Icon aria-hidden className="size-6" />
                  {item.label}
                </Link>
              </li>
            )
          })}
        </ul>
      </nav>
      <nav aria-label={t('nav.more')} className="border-t border-border pt-4">
        <ul className="grid gap-1">
          <li>
            <Link
              to="/notifications"
              aria-current={inNotifications ? 'page' : undefined}
              className={cn(LINK, inNotifications && LINK_ACTIVE)}
            >
              {inNotifications ? (
                <IconBellFill aria-hidden className="size-6" />
              ) : (
                <IconBell aria-hidden className="size-6" />
              )}
              {t('notifications.title')}
              {badge && (
                <span className="ml-auto rounded-full bg-destructive px-1.5 text-2xs font-semibold text-white tabular-nums">
                  {badge}
                </span>
              )}
            </Link>
          </li>
          {canUpload &&
            MORE.map((item) => (
              <li key={item.to}>
                <Link
                  to={item.to}
                  className={cn(
                    LINK,
                    'data-[status=active]:font-semibold data-[status=active]:text-accent-ink',
                  )}
                >
                  {({ isActive }) => {
                    const Icon = isActive ? item.activeIcon : item.icon
                    return (
                      <>
                        <Icon aria-hidden className="size-6" />
                        {t(item.label)}
                      </>
                    )
                  }}
                </Link>
              </li>
            ))}
        </ul>
      </nav>
      {canUpload && (
        <div className="mt-auto">
          <Button onClick={onAdd} aria-haspopup="dialog" className="w-full">
            <IconAdd aria-hidden className="size-5" />
            {t('addSheet.open')}
          </Button>
        </div>
      )}
    </div>
  )
}
