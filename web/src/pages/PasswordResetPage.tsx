import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate, useParams } from '@tanstack/react-router'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { api, call } from '@/api/client'
import { ApiError } from '@/api/errors'
import { clearPreviousUserData, setSignedIn } from '@/auth/session'
import { FormError } from '@/components/Field'
import { PasswordField } from '@/components/PasswordField'
import { Button } from '@/components/ui/button'
import { errorMessage } from '@/i18n'
import { AuthLayout } from './AuthLayout'
import { PasswordRules } from './authParts'

const INVALID_LINK_KEYS = ['expired', 'used', 'revoked', 'notFound'] as const

// 이유별 문구가 있는 서버 키(passwordReset.*). 없으면 null.
function invalidLinkReason(
  e: unknown,
): (typeof INVALID_LINK_KEYS)[number] | null {
  if (!(e instanceof ApiError)) return null
  return INVALID_LINK_KEYS.find((k) => e.key === `passwordReset.${k}`) ?? null
}

export function PasswordResetPage() {
  const { t } = useTranslation()
  const { raw } = useParams({ from: '/reset/$raw' })
  const navigate = useNavigate()
  const qc = useQueryClient()
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  // 확인 칸 blur 또는 제출 시도 뒤부터 불일치 표시.
  const [confirmTouched, setConfirmTouched] = useState(false)
  const preview = useQuery({
    queryKey: ['password-reset', raw],
    queryFn: () =>
      call(
        api.GET('/api/password-resets/{raw}/preview', {
          params: { path: { raw } },
        }),
      ),
    retry: false,
  })
  const accept = useMutation({
    mutationFn: () =>
      call(
        api.POST('/api/password-resets/{raw}/accept', {
          params: { path: { raw } },
          body: { password },
        }),
      ),
    onSuccess: async ({ user }) => {
      // 미리보기 쿼리는 유지. 지우면 재조회가 410(used)을 받아 안내 화면이 번쩍임.
      clearPreviousUserData(qc, 'password-reset')
      setSignedIn(qc, user)
      toast(t('auth.reset.welcome'))
      await navigate({ to: '/' })
    },
  })

  if (preview.isPending) return <AuthLayout title={t('common.loading')} />
  // 미리보기 실패 또는 입력 중 링크 무효화(만료, 회수, 동시 사용).
  const linkError = preview.isError
    ? preview.error
    : invalidLinkReason(accept.error)
      ? accept.error
      : null
  if (linkError) {
    const reason = invalidLinkReason(linkError)
    return (
      <AuthLayout
        title={
          reason ? t(`auth.reset.error.${reason}`) : errorMessage(t, linkError)
        }
        description={reason ? t('auth.reset.action') : null}
      >
        <Link
          to="/login"
          className="text-sm font-semibold text-accent-ink underline"
        >
          {t('auth.toLogin')}
        </Link>
      </AuthLayout>
    )
  }

  if (!preview.data) return <AuthLayout title={t('common.loading')} />
  const { display_name, username } = preview.data
  const mismatch = confirmTouched && confirm !== password
  return (
    <AuthLayout
      title={t('auth.reset.title', { name: display_name })}
      description={
        <>
          <p>{t('auth.reset.username', { username })}</p>
          <p>{t('auth.reset.hint')}</p>
        </>
      }
    >
      <form
        className="grid gap-4"
        onSubmit={(e) => {
          e.preventDefault()
          if (confirm !== password) {
            setConfirmTouched(true)
            e.currentTarget
              .querySelector<HTMLInputElement>('[name="password_confirm"]')
              ?.focus()
            return
          }
          accept.mutate()
        }}
      >
        <PasswordField
          name="password"
          label={t('auth.password')}
          autoComplete="new-password"
          required
          autoFocus
          value={password}
          onChange={(e) => {
            setPassword(e.currentTarget.value)
          }}
        />
        <PasswordField
          confirm
          name="password_confirm"
          label={t('auth.passwordConfirm')}
          autoComplete="new-password"
          required
          value={confirm}
          error={mismatch ? t('auth.passwordMismatch') : null}
          onChange={(e) => {
            setConfirm(e.currentTarget.value)
          }}
          onBlur={() => {
            setConfirmTouched(true)
          }}
        />
        <PasswordRules password={password} />
        <FormError error={accept.error} />
        <Button type="submit" disabled={accept.isPending}>
          {t('auth.reset.submit')}
        </Button>
      </form>
    </AuthLayout>
  )
}
