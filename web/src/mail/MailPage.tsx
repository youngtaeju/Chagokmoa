import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { FormError } from '@/components/Field'
import { SubPage } from '@/components/SubPage'
import { Button } from '@/components/ui/button'
import { MailSection } from './MailSection'
import { mailQuery } from './useMail'

export function MailPage() {
  const { t } = useTranslation()
  const mail = useQuery(mailQuery)
  return (
    <SubPage section={t('me.system')} title={t('me.mail')}>
      <div className="mx-auto w-full max-w-xl">
        {mail.data ? (
          <MailSection config={mail.data.config} />
        ) : mail.error ? (
          <div className="grid justify-items-start gap-3">
            <FormError error={mail.error} />
            <Button
              variant="outline"
              onClick={() => {
                void mail.refetch()
              }}
            >
              {t('common.retry')}
            </Button>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">{t('common.loading')}</p>
        )}
      </div>
    </SubPage>
  )
}
