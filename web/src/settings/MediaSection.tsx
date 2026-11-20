import { useTranslation } from 'react-i18next'
import type { Settings } from '@/api/types'
import { Field, FormError, formText } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { useSaveSettings } from './useSettings'

export function MediaSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const save = useSaveSettings()

  return (
    <form
      className="grid gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        const form = new FormData(e.currentTarget)
        const avif = form.get('derivatives_avif') === 'on'
        const retention = Number(formText(form, 'trash_retention_days'))
        const body: Partial<Settings> = {}
        if (avif !== settings.derivatives_avif) body.derivatives_avif = avif
        if (retention !== settings.trash_retention_days) {
          body.trash_retention_days = retention
        }
        if (Object.keys(body).length > 0) save.mutate(body)
      }}
    >
      <div className="grid gap-1">
        <div className="flex items-center gap-2">
          <input
            id="derivatives-avif"
            name="derivatives_avif"
            type="checkbox"
            className="size-4 accent-primary"
            defaultChecked={settings.derivatives_avif}
          />
          <Label htmlFor="derivatives-avif">{t('settings.avif')}</Label>
        </div>
        <p className="text-xs text-muted-foreground">
          {t('settings.avifHint')}
        </p>
      </div>
      <Field
        label={t('settings.trashRetention')}
        hint={t('settings.trashRetentionHint')}
        name="trash_retention_days"
        type="number"
        inputMode="numeric"
        min={0}
        max={3650}
        step={1}
        defaultValue={settings.trash_retention_days}
        required
      />
      <FormError error={save.error} />
      <div>
        <Button type="submit" disabled={save.isPending}>
          {t('common.save')}
        </Button>
      </div>
    </form>
  )
}
