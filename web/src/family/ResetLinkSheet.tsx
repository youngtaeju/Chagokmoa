import { useMutation, useQuery } from '@tanstack/react-query'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { api, call } from '@/api/client'
import type { User } from '@/api/types'
import { authStatusQuery } from '@/auth/session'
import { FormError } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { useSettings } from '@/settings/useSettings'
import { AdaptiveSheet } from '@/upload/AdaptiveSheet'
import { LinkShareBox, MailResult } from './LinkShareBox'

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
  const mailEnabled = useQuery(authStatusQuery).data?.mail_enabled === true
  // 메일 설정됨이고 대상 이메일 있을 때만 발송 선택.
  const mailTo = mailEnabled ? user.email : null
  const [sendEmail, setSendEmail] = useState(true)
  const checkId = useId()
  const create = useMutation({
    mutationFn: (send_email: boolean) =>
      call(
        api.POST('/api/users/{id}/password-reset', {
          params: { path: { id: user.id } },
          body: { send_email },
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
        {mailTo && (
          <div className="flex items-center gap-2">
            <Checkbox
              id={checkId}
              checked={sendEmail}
              onCheckedChange={(v) => {
                setSendEmail(v === true)
              }}
            />
            <Label htmlFor={checkId}>
              {t('members.resetSendEmail', { email: mailTo })}
            </Label>
          </div>
        )}
        <Button
          disabled={create.isPending}
          onClick={() => {
            create.mutate(mailTo !== null && sendEmail)
          }}
        >
          {t('members.createReset')}
        </Button>
        <FormError error={create.error} />
        {mailTo && create.data && create.data.mail_sent !== null && (
          <MailResult email={mailTo} sent={create.data.mail_sent} />
        )}
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
