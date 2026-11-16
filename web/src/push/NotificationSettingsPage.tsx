import { useId, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { FormError } from '@/components/Field'
import { Switch } from '@/components/ui/switch'
import { MeSubPage } from '@/me/MeSubPage'
import { usePushDevice } from './usePushDevice'
import { useSetPushPref, usePushPrefs, type PushKind } from './usePushPrefs'

const KINDS: PushKind[] = ['upload', 'comment', 'mention', 'story', 'milestone']

const ROW = 'flex min-h-14 items-center gap-3 px-4 text-[15px]'

// 나 화면과 같은 규칙: 그룹 위 실선, 진한 라벨.
function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-t border-border pt-3 pb-2">
      <h2 className="px-4 pb-1 text-[13px] font-bold">{title}</h2>
      {children}
    </section>
  )
}

function DeviceGroup() {
  const { t } = useTranslation()
  const { support, on, pending, enable, disable } = usePushDevice()
  const [error, setError] = useState<unknown>(null)
  const labelId = useId()
  const hintId = useId()
  const hint =
    support === 'ios-install'
      ? t('push.iosInstall')
      : support === 'denied'
        ? t('push.denied')
        : support === 'unsupported'
          ? t('push.unsupported')
          : null
  const permissionError =
    error instanceof Error && error.message === 'push.permissionDenied'
  return (
    <Group title={t('push.device')}>
      <div className={ROW}>
        <span id={labelId} className="flex-1">
          {t('push.deviceToggle')}
        </span>
        {/* on === null: 이 기기 구독 확인 중. */}
        <Switch
          aria-labelledby={labelId}
          aria-describedby={hint ? hintId : undefined}
          checked={on === true}
          disabled={support !== 'supported' || pending || on === null}
          onCheckedChange={(next) => {
            setError(null)
            const onError = (e: unknown) => {
              setError(e)
            }
            if (next) {
              enable().catch(onError)
            } else {
              disable().catch(onError)
            }
          }}
        />
      </div>
      <div className="grid gap-1 px-4">
        {hint && (
          <p id={hintId} className="text-sm text-muted-foreground">
            {hint}
          </p>
        )}
        {permissionError ? (
          // 권한 창을 닫아 default 로 남은 경우만. denied 면 위 안내 하나로.
          support !== 'denied' && (
            <p role="alert" className="text-sm text-destructive">
              {t('push.permissionDenied')}
            </p>
          )
        ) : (
          <FormError error={error} />
        )}
      </div>
    </Group>
  )
}

function KindsGroup() {
  const { t } = useTranslation()
  const prefs = usePushPrefs()
  const set = useSetPushPref()
  const id = useId()
  return (
    <Group title={t('push.kinds')}>
      {KINDS.map((k) => (
        <div key={k} className={ROW}>
          <span id={`${id}-${k}`} className="flex-1">
            {t(`push.kind.${k}`)}
          </span>
          <Switch
            aria-labelledby={`${id}-${k}`}
            checked={prefs.data?.[k] ?? false}
            disabled={!prefs.data}
            onCheckedChange={(next) => {
              set.mutate({ [k]: next })
            }}
          />
        </div>
      ))}
      <div className="grid gap-1 px-4">
        <FormError error={prefs.error ?? set.error} />
        <p className="text-sm text-muted-foreground">{t('push.listNote')}</p>
      </div>
    </Group>
  )
}

export function NotificationSettingsPage() {
  const { t } = useTranslation()
  return (
    <MeSubPage title={t('push.title')}>
      {/* 휴대폰은 실선이 화면 끝까지(-mx-4), 넓은 화면은 가운데 열. */}
      <div className="-mx-4 grid sm:mx-auto sm:max-w-xl">
        <DeviceGroup />
        <KindsGroup />
      </div>
    </MeSubPage>
  )
}
