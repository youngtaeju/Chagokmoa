import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import IconChevron from '~icons/material-symbols/chevron-right-rounded'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { authStatusQuery, hasRole, useMe } from '@/auth/session'
import { useSignOut } from '@/auth/useSignOut'
import { cn } from '@/lib/utils'

type RowPath =
  | '/settings/profile'
  | '/settings/password'
  | '/settings/devices'
  | '/settings/notifications'
  | '/settings/appearance'
  | '/settings/trash'
  | '/settings/general'
  | '/settings/media'
  | '/settings/backup'
  | '/settings/mail'
  | '/settings/login'
  | '/settings/login-method'
  | '/family'

const ROW =
  'flex min-h-14 w-full items-center gap-3 px-4 text-left text-[15px] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring'

// 그룹 사이 화면 폭 실선, 라벨은 그룹 첫 줄.
function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-t border-border pt-3 pb-2">
      <h2 className="px-4 pb-1 text-[13px] font-bold text-foreground">
        {title}
      </h2>
      <ul>{children}</ul>
    </section>
  )
}

function NavRow({ to, label }: { to: RowPath; label: string }) {
  return (
    <li>
      <Link to={to} className={ROW}>
        <span className="flex-1">{label}</span>
        <IconChevron aria-hidden className="size-6 text-muted-foreground" />
      </Link>
    </li>
  )
}

export function SettingsPage() {
  const { t } = useTranslation()
  const { user } = useMe()
  const signOut = useSignOut()
  const status = useQuery(authStatusQuery)
  const initial = Array.from(user?.display_name ?? '')[0] ?? '?'

  return (
    // 휴대폰은 실선이 화면 끝까지(-mx-4), 넓은 화면은 가운데 열.
    <div className="-mx-4 grid sm:mx-auto sm:max-w-xl">
      {user && (
        <Link to="/settings/profile" className={cn(ROW, 'mb-3 min-h-20 gap-4')}>
          <span
            aria-hidden
            className="inline-flex size-12 shrink-0 items-center justify-center rounded-full bg-accent-soft text-xl font-bold text-accent-ink"
          >
            {initial}
          </span>
          <span className="grid min-w-0 flex-1 gap-0.5">
            <h1 className="truncate text-xl font-bold">{user.display_name}</h1>
            <span className="truncate text-[13px] text-muted-foreground">
              @{user.username}
            </span>
          </span>
          <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-xs font-semibold">
            {t(`roles.${user.role}`)}
          </span>
          <IconChevron aria-hidden className="size-6 text-muted-foreground" />
        </Link>
      )}
      <Group title={t('me.account')}>
        <NavRow to="/settings/profile" label={t('me.profile')} />
        <NavRow to="/settings/password" label={t('me.password')} />
        <NavRow to="/settings/devices" label={t('me.devices')} />
        {status.data?.oidc && (
          <NavRow to="/settings/login-method" label={t('me.loginMethod')} />
        )}
      </Group>
      <Group title={t('me.app')}>
        <NavRow to="/settings/notifications" label={t('me.notifications')} />
        <NavRow to="/settings/appearance" label={t('me.appearance')} />
      </Group>
      <Group title={t('me.home')}>
        <NavRow to="/family" label={t('me.family')} />
        {hasRole(user, 'guardian') && (
          <NavRow to="/settings/trash" label={t('me.trash')} />
        )}
      </Group>
      {hasRole(user, 'owner') && (
        <Group title={t('me.system')}>
          <NavRow to="/settings/general" label={t('me.general')} />
          <NavRow to="/settings/media" label={t('me.media')} />
          <NavRow to="/settings/backup" label={t('me.backup')} />
          <NavRow to="/settings/mail" label={t('me.mail')} />
          <NavRow to="/settings/login" label={t('me.login')} />
        </Group>
      )}
      <section className="grid gap-3 border-t border-border pt-2">
        <button
          type="button"
          className={cn(ROW, 'text-destructive')}
          onClick={() => {
            void signOut()
          }}
        >
          {t('me.logout')}
        </button>
        <p className="text-center text-xs text-muted-foreground tabular-nums">
          {t('me.version', { version: __APP_VERSION__ })}
        </p>
      </section>
    </div>
  )
}
