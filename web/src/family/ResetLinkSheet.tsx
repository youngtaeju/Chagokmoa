import { useMutation } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { api, call } from '@/api/client'
import type { User } from '@/api/types'
import { FormError } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { useSettings } from '@/settings/useSettings'
import { AdaptiveSheet } from '@/upload/AdaptiveSheet'
import { LinkShareBox } from './LinkShareBox'

// owner 전용. 열 때는 만들지 않음(새 링크가 이전 링크를 무효화).
export function ResetLinkSheet({
  user,
  onClose,
}: {
  user: User
  onClose: () => void
}) {
  const { t } = useTranslation()
  const instance = useSettings()?.instance_name
  const create = useMutation({
    mutationFn: () =>
      call(
        api.POST('/api/users/{id}/password-reset', {
          params: { path: { id: user.id } },
        }),
      ),
  })

  return (
    <AdaptiveSheet
      title={t('members.resetSheetTitle', { name: user.display_name })}
      onClose={onClose}
    >
      <div className="grid gap-4 pt-2">
        <p className="text-sm text-muted-foreground">
          {t('members.resetInfo')}
        </p>
        <Button
          disabled={create.isPending}
          onClick={() => {
            create.mutate()
          }}
        >
          {t('members.createReset')}
        </Button>
        <FormError error={create.error} />
        {create.data && (
          <LinkShareBox
            url={create.data.url}
            label={t('members.resetUrl')}
            shareTitle={
              instance
                ? t('members.resetShareTitle', { instance })
                : t('members.resetShareTitleGeneric')
            }
            shareText={
              instance
                ? t('members.resetShareText', { instance })
                : t('members.resetShareTextGeneric')
            }
            hint={t('members.resetUrlHint')}
          />
        )}
      </div>
    </AdaptiveSheet>
  )
}
