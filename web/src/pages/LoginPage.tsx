import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useSearch } from '@tanstack/react-router'
import type { ReactNode } from 'react'
import { Trans, useTranslation } from 'react-i18next'
import { api, call } from '@/api/client'
import { authStatusQuery, setSignedIn } from '@/auth/session'
import { Field, FormError, formText } from '@/components/Field'
import { PasswordField } from '@/components/PasswordField'
import { Button } from '@/components/ui/button'
import { OidcErrorNotice, useOidcError } from '@/oidc/useOidcError'
import IconLock from '~icons/material-symbols/lock-outline-rounded'
import IconLogin from '~icons/material-symbols/login-rounded'
import IconPerson from '~icons/material-symbols/person-outline-rounded'
import { ForgotPasswordSheet } from './ForgotPasswordSheet'
import { AuthLayout } from './AuthLayout'

// 강조어 em. Trans 가 em 자식을 번역문으로 교체하므로 밑줄은 컴포넌트가 그림.
function Underlined({ children }: { children?: ReactNode }) {
  return (
    <em className="relative inline-block font-bold not-italic text-auth-brand">
      {children}
      <svg
        aria-hidden="true"
        viewBox="0 0 80 10"
        preserveAspectRatio="none"
        className="absolute -bottom-[5px] -left-0.5 h-[9px] w-[calc(100%+4px)]"
      >
        <path
          d="M2 7 C 20 3, 46 2, 78 5"
          fill="none"
          stroke="currentColor"
          strokeWidth="3"
          strokeLinecap="round"
          className="text-primary"
        />
      </svg>
    </em>
  )
}

// 강조어 손그림 밑줄과 끝 하트 장식. 문구는 번역 문자열의 br, em.
function Tagline() {
  return (
    <>
      <Trans
        i18nKey="auth.login.tagline"
        components={{
          br: <br />,
          em: <Underlined />,
        }}
      />
      <svg
        aria-hidden="true"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.2"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="ml-0.5 inline size-[18px] align-[-3px] text-primary"
      >
        <path
          d="M12 19 C 5 14, 3 10.5, 4.5 7.5 C 6 4.8, 10 5, 12 8.5 C 14 5, 18 4.8, 19.5 7.5 C 21 10.5, 19 14, 12 19 Z"
          transform="rotate(12 12 12)"
        />
      </svg>
    </>
  )
}

export function LoginPage() {
  const { t } = useTranslation()
  const { redirect } = useSearch({ from: '/login' })
  const oidcError = useOidcError()
  const navigate = useNavigate()
  const qc = useQueryClient()
  const status = useQuery(authStatusQuery)
  const login = useMutation({
    mutationFn: (f: FormData) =>
      call(
        api.POST('/api/auth/login', {
          body: {
            username: formText(f, 'username'),
            password: formText(f, 'password'),
          },
        }),
      ),
    onSuccess: async ({ user }) => {
      setSignedIn(qc, user)
      await navigate({ href: redirect ?? '/' })
    },
  })

  return (
    <AuthLayout intro={<Tagline />}>
      <OidcErrorNotice message={oidcError} />
      <form
        className="grid gap-4"
        onSubmit={(e) => {
          e.preventDefault()
          login.mutate(new FormData(e.currentTarget))
        }}
      >
        <Field
          label={t('auth.username')}
          name="username"
          autoComplete="username"
          autoCapitalize="none"
          icon={<IconPerson />}
          tall
          required
        />
        <PasswordField
          label={t('auth.password')}
          name="password"
          autoComplete="current-password"
          icon={<IconLock />}
          tall
          required
        />
        <FormError error={login.error} />
        <Button
          type="submit"
          disabled={login.isPending}
          className="mt-2 h-[52px] text-[17px] font-bold [&_svg:not([class*='size-'])]:size-5"
        >
          <IconLogin aria-hidden="true" />
          {t('auth.login.submit')}
        </Button>
      </form>
      {status.data?.oidc && (
        <div className="grid gap-4">
          <div
            role="separator"
            className="flex items-center gap-3 text-sm text-muted-foreground before:h-px before:flex-1 before:bg-border after:h-px after:flex-1 after:bg-border"
          >
            {t('auth.login.or')}
          </div>
          <Button
            asChild
            variant="outline"
            className="h-[52px] text-[17px] font-bold"
          >
            <a
              href={`/api/auth/oidc/start?purpose=login${
                redirect ? `&redirect=${encodeURIComponent(redirect)}` : ''
              }`}
            >
              {t('auth.login.oidc', { label: status.data.oidc.label })}
            </a>
          </Button>
        </div>
      )}
      <div className="-mt-3 flex flex-col items-center gap-0.5">
        <ForgotPasswordSheet />
        {status.data?.signup_open && (
          <p className="text-[15px] text-muted-foreground">
            {t('auth.login.firstRun')}{' '}
            <Link
              to="/signup"
              className="font-bold text-accent-ink underline underline-offset-4"
            >
              {t('auth.login.toSignup')}
            </Link>
          </p>
        )}
      </div>
    </AuthLayout>
  )
}
