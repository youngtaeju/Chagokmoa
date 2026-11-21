import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { FormError } from '@/components/Field'
import { SubPage } from '@/components/SubPage'
import { Button } from '@/components/ui/button'
import { OidcSection } from './OidcSection'
import { oidcQuery } from './useOidc'

export function OidcPage() {
  const { t } = useTranslation()
  const oidc = useQuery(oidcQuery)
  return (
    <SubPage section={t('me.system')} title={t('me.login')}>
      <div className="mx-auto w-full max-w-xl">
        {oidc.data ? (
          <OidcSection res={oidc.data} />
        ) : oidc.error ? (
          <div className="grid justify-items-start gap-3">
            <FormError error={oidc.error} />
            <Button
              variant="outline"
              onClick={() => {
                void oidc.refetch()
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
