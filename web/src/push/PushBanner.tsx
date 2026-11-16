import IconClose from '~icons/material-symbols/close-rounded'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { FormError } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { usePushDevice } from './usePushDevice'

const DISMISSED_KEY = 'cgm.push.banner.dismissed'

function wasDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISSED_KEY) === '1'
  } catch {
    return false
  }
}

// 알림 화면 위 안내. 켤 수 있는 기기에서만, 닫으면 기억.
export function PushBanner() {
  const { t } = useTranslation()
  const { support, on, pending, enable } = usePushDevice()
  const [dismissed, setDismissed] = useState(wasDismissed)
  const [error, setError] = useState<unknown>(null)
  // on === null: 이 기기 구독 확인 중. 켜져 있을 수 있어 숨김.
  if (support !== 'supported' || on !== false || dismissed) return null
  return (
    <div className="grid gap-2 rounded-xl bg-accent-soft px-4 py-3">
      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 text-sm text-accent-ink">
          {t('push.banner')}
        </p>
        <Button
          size="sm"
          disabled={pending}
          onClick={() => {
            setError(null)
            enable().catch((e: unknown) => {
              // 권한 거부: denied 면 띠가 사라지고, 창을 닫아 default 면 띠 유지(문구 없음).
              if (!(
                e instanceof Error && e.message === 'push.permissionDenied'
              ))
                setError(e)
            })
          }}
        >
          {t('push.bannerOn')}
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t('push.bannerClose')}
          onClick={() => {
            try {
              localStorage.setItem(DISMISSED_KEY, '1')
            } catch {
              // 저장 불가여도 이번 방문은 닫음.
            }
            setDismissed(true)
          }}
        >
          <IconClose aria-hidden className="size-5" />
        </Button>
      </div>
      <FormError error={error} />
    </div>
  )
}
