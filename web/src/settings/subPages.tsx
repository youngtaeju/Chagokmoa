import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { meQuery } from '@/auth/session'
import { FormError } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { AppearanceSection } from '@/settings/AppearanceSection'
import { InstanceSection } from '@/settings/InstanceSection'
import { PasswordSection } from '@/settings/PasswordSection'
import { ProfileSection } from '@/settings/ProfileSection'
import { SessionsSection } from '@/settings/SessionsSection'
import { useSettingsQuery } from '@/settings/useSettings'
import { SubPage } from '@/components/SubPage'

function Body({ children }: { children: ReactNode }) {
  return <div className="mx-auto w-full max-w-xl">{children}</div>
}

// 조회 중 안내, 실패 시 오류와 다시 시도.
function Loaded<T>({
  query,
  children,
}: {
  query: {
    data: T | undefined
    error: Error | null
    refetch: () => unknown
  }
  children: (data: T) => ReactNode
}) {
  const { t } = useTranslation()
  if (query.data !== undefined) return <>{children(query.data)}</>
  if (query.error) {
    return (
      <div className="grid justify-items-start gap-3">
        <FormError error={query.error} />
        <Button
          variant="outline"
          onClick={() => {
            void query.refetch()
          }}
        >
          {t('common.retry')}
        </Button>
      </div>
    )
  }
  return <p className="text-sm text-muted-foreground">{t('common.loading')}</p>
}

export function ProfilePage() {
  const { t } = useTranslation()
  const me = useQuery(meQuery)
  return (
    <SubPage title={t('me.profile')}>
      <Body>
        <Loaded query={{ ...me, data: me.data ?? undefined }}>
          {(user) => <ProfileSection user={user} />}
        </Loaded>
      </Body>
    </SubPage>
  )
}

export function PasswordPage() {
  const { t } = useTranslation()
  return (
    <SubPage title={t('me.password')}>
      <Body>
        <PasswordSection />
      </Body>
    </SubPage>
  )
}

export function DevicesPage() {
  const { t } = useTranslation()
  return (
    <SubPage title={t('me.devices')}>
      <Body>
        <SessionsSection />
      </Body>
    </SubPage>
  )
}

export function AppearancePage() {
  const { t } = useTranslation()
  const me = useQuery(meQuery)
  return (
    <SubPage title={t('me.appearance')}>
      <Body>
        <Loaded query={{ ...me, data: me.data ?? undefined }}>
          {(user) => <AppearanceSection user={user} />}
        </Loaded>
      </Body>
    </SubPage>
  )
}

export function InstancePage() {
  const { t } = useTranslation()
  const settings = useSettingsQuery()
  return (
    <SubPage title={t('me.instance')}>
      <Body>
        <Loaded query={settings}>
          {(s) => <InstanceSection settings={s} />}
        </Loaded>
      </Body>
    </SubPage>
  )
}
