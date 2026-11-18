import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useId } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { api, call } from '@/api/client'
import type { BackupStatus } from '@/api/types'
import type { components } from '@/api/schema'
import { Field, FormError, formText } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { backupKey } from './status'

type UpdateBackup = components['schemas']['UpdateBackupRequest']

// 서버 상한(routes.rs DAILY_MAX 등)과 같음.
const RETENTION_FIELDS = [
  ['daily', 365],
  ['weekly', 520],
  ['monthly', 240],
] as const

// 자동 백업 켜기, 시각, 보관 정책.
export function SettingsSection({ status }: { status: BackupStatus }) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const titleId = useId()
  const toggleId = useId()
  const update = (body: UpdateBackup) => call(api.PUT('/api/backup', { body }))
  const toggle = useMutation({
    mutationFn: (enabled: boolean) => update({ enabled }),
    onSuccess: (s) => {
      qc.setQueryData(backupKey, s)
    },
  })
  const save = useMutation({
    mutationFn: update,
    onSuccess: (s) => {
      qc.setQueryData(backupKey, s)
      toast(t('backup.settingsSaved'))
    },
  })

  return (
    <section
      aria-labelledby={titleId}
      className="grid gap-3 border-t border-border pt-4"
    >
      <h2 id={titleId} className="text-base font-bold">
        {t('backup.settingsTitle')}
      </h2>
      <div className="flex min-h-11 items-center gap-3">
        <span id={toggleId} className="flex-1 text-sm">
          {t('backup.autoToggle')}
        </span>
        <Switch
          aria-labelledby={toggleId}
          checked={status.enabled}
          disabled={toggle.isPending}
          onCheckedChange={(next) => {
            toggle.mutate(next)
          }}
        />
      </div>
      <FormError error={toggle.error} />
      <form
        // 저장값이 바뀌면 입력칸 다시 채움.
        key={[
          status.time,
          status.retention.daily,
          status.retention.weekly,
          status.retention.monthly,
        ].join('/')}
        className="grid gap-3"
        onSubmit={(e) => {
          e.preventDefault()
          const form = new FormData(e.currentTarget)
          const time = formText(form, 'time')
          const retention = {
            daily: Number(formText(form, 'daily')),
            weekly: Number(formText(form, 'weekly')),
            monthly: Number(formText(form, 'monthly')),
          }
          const body: UpdateBackup = {}
          if (time !== status.time) body.time = time
          if (
            RETENTION_FIELDS.some(([k]) => retention[k] !== status.retention[k])
          ) {
            body.retention = retention
          }
          if (Object.keys(body).length > 0) save.mutate(body)
        }}
      >
        <Field
          label={t('backup.time')}
          hint={t('backup.timeHint')}
          name="time"
          type="time"
          step={60}
          defaultValue={status.time}
          required
        />
        <fieldset className="grid gap-2">
          <legend className="text-sm font-medium">
            {t('backup.retention')}
          </legend>
          <div className="grid gap-3 sm:grid-cols-3">
            {RETENTION_FIELDS.map(([k, max]) => (
              <Field
                key={k}
                label={t(`backup.${k}`)}
                name={k}
                type="number"
                inputMode="numeric"
                min={0}
                max={max}
                step={1}
                defaultValue={status.retention[k]}
                required
              />
            ))}
          </div>
          <p className="text-xs text-muted-foreground">
            {t('backup.retentionHint')}
          </p>
        </fieldset>
        <FormError error={save.error} />
        <div>
          <Button type="submit" disabled={save.isPending}>
            {t('common.save')}
          </Button>
        </div>
      </form>
    </section>
  )
}
