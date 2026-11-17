import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { api, call } from '@/api/client'
import type { Role } from '@/api/types'
import { FormError } from '@/components/Field'
import { SelectField } from '@/components/NativeSelect'
import { Button } from '@/components/ui/button'
import { LinkShareBox } from './LinkShareBox'
import { useSettings } from '@/settings/useSettings'
import { AdaptiveSheet } from '@/upload/AdaptiveSheet'

export const invitesKey = ['invites'] as const
const INVITE_ROLES = ['guardian', 'family'] as const
const EXPIRY_HOURS = [24, 72, 168] as const

function isInviteRole(v: string): v is (typeof INVITE_ROLES)[number] {
  return v === 'guardian' || v === 'family'
}

export function InviteSheet({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const instance = useSettings()?.instance_name
  const [url, setUrl] = useState<string | null>(null)

  const create = useMutation({
    mutationFn: (body: { role: Role; expires_in_hours: number }) =>
      call(api.POST('/api/invites', { body })),
    onSuccess: (res) => {
      setUrl(res.url)
      void qc.invalidateQueries({ queryKey: invitesKey })
    },
  })

  return (
    <AdaptiveSheet title={t('members.inviteSheetTitle')} onClose={onClose}>
      <div className="grid gap-4 pt-2">
        <form
          className="grid gap-3"
          onSubmit={(e) => {
            e.preventDefault()
            const form = new FormData(e.currentTarget)
            const role = form.get('role')
            const hours = Number(form.get('hours'))
            if (typeof role === 'string' && isInviteRole(role)) {
              create.mutate({ role, expires_in_hours: hours })
            }
          }}
        >
          <SelectField
            label={t('members.inviteRole')}
            name="role"
            defaultValue="family"
          >
            {INVITE_ROLES.map((r) => (
              <option key={r} value={r}>
                {t(`roles.${r}`)} · {t(`roles.${r}Desc`)}
              </option>
            ))}
          </SelectField>
          <SelectField
            label={t('members.inviteExpiry')}
            name="hours"
            defaultValue="72"
          >
            {EXPIRY_HOURS.map((h) => (
              <option key={h} value={h}>
                {t(`members.expiry.${String(h)}`)}
              </option>
            ))}
          </SelectField>
          <Button type="submit" disabled={create.isPending}>
            {t('members.createInvite')}
          </Button>
        </form>
        <FormError error={create.error} />
        {url !== null && (
          <LinkShareBox
            url={url}
            label={t('members.inviteUrl')}
            shareTitle={
              instance
                ? t('members.shareTitle', { instance })
                : t('members.shareTitleGeneric')
            }
            shareText={
              instance
                ? t('members.shareText', { instance })
                : t('members.shareTextGeneric')
            }
            hint={t('members.inviteUrlHint')}
          />
        )}
      </div>
    </AdaptiveSheet>
  )
}
