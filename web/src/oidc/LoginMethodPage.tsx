import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { api, call } from '@/api/client'
import { authStatusQuery } from '@/auth/session'
import { FormError } from '@/components/Field'
import { SubPage } from '@/components/SubPage'
import { Button } from '@/components/ui/button'
import { loginMethodKey } from '@/settings/PasswordSection'
import { OidcErrorNotice, useOidcError } from './useOidcError'

export function LoginMethodPage() {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const redirectError = useOidcError()
  const status = useQuery(authStatusQuery)
  const method = useQuery({
    queryKey: loginMethodKey,
    queryFn: () => call(api.GET('/api/auth/login-method')),
  })
  const unlink = useMutation({
    mutationFn: () => call(api.DELETE('/api/auth/oidc/link')),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: loginMethodKey })
      toast(t('loginMethod.unlinked'))
    },
  })
  const label = status.data?.oidc?.label
  const state = method.data?.oidc

  return (
    <SubPage section={t('me.account')} title={t('me.loginMethod')}>
      <div className="mx-auto grid w-full max-w-xl gap-4">
        <OidcErrorNotice message={redirectError} />
        {method.error || status.error ? (
          <div className="grid justify-items-start gap-3">
            <FormError error={method.error ?? status.error} />
            <Button
              variant="outline"
              onClick={() => {
                void method.refetch()
                void status.refetch()
              }}
            >
              {t('common.retry')}
            </Button>
          </div>
        ) : status.data && !status.data.oidc ? (
          <p className="text-sm text-muted-foreground">
            {t('errors.oidc.disabled')}
          </p>
        ) : !method.data || !label ? (
          <p className="text-sm text-muted-foreground">{t('common.loading')}</p>
        ) : state === 'linked' ? (
          <div className="grid justify-items-start gap-3">
            <p className="font-semibold">{t('loginMethod.linked')}</p>
            <Button
              variant="outline"
              disabled={!method.data.has_password || unlink.isPending}
              onClick={() => {
                unlink.mutate()
              }}
            >
              {t('loginMethod.unlink')}
            </Button>
            {!method.data.has_password && (
              <p className="text-sm text-muted-foreground">
                {t('errors.oidc.needPassword')}{' '}
                <Link
                  to="/settings/password"
                  className="font-semibold text-accent-ink underline underline-offset-4"
                >
                  {t('me.password')}
                </Link>
              </p>
            )}
            <FormError error={unlink.error} />
          </div>
        ) : (
          <div className="grid justify-items-start gap-3">
            <p className="font-semibold">
              {t(state === 'stale' ? 'loginMethod.stale' : 'loginMethod.none')}
            </p>
            <Button asChild>
              <a href="/api/auth/oidc/start?purpose=link">
                {t('loginMethod.connect', { label })}
              </a>
            </Button>
          </div>
        )}
      </div>
    </SubPage>
  )
}
