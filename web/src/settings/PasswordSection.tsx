import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { api, call } from '@/api/client'
import { ApiError } from '@/api/errors'
import { Field, FormError, formText } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { sessionsKey } from './SessionsSection'

export const loginMethodKey = ['login-method'] as const

export function PasswordSection() {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const method = useQuery({
    queryKey: loginMethodKey,
    queryFn: () => call(api.GET('/api/auth/login-method')),
  })
  const change = useMutation({
    mutationFn: (body: { current_password?: string; new_password: string }) =>
      call(api.POST('/api/users/me/password', { body })),
    onSuccess: (_, body) => {
      // 서버가 다른 세션을 모두 삭제하므로 목록 재조회.
      void qc.invalidateQueries({ queryKey: sessionsKey })
      void qc.invalidateQueries({ queryKey: loginMethodKey })
      toast(
        t(
          body.current_password === undefined
            ? 'settings.passwordCreated'
            : 'settings.passwordChanged',
        ),
      )
    },
  })

  // 비밀번호 유무 확인 전에는 폼 숨김(잘못된 칸 노출 방지).
  if (!method.data) {
    return method.error ? (
      <div className="grid justify-items-start gap-3">
        <FormError error={method.error} />
        <Button
          variant="outline"
          onClick={() => {
            void method.refetch()
          }}
        >
          {t('common.retry')}
        </Button>
      </div>
    ) : null
  }
  const hasPassword = method.data.has_password

  return (
    <form
      className="grid gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        const form = e.currentTarget
        const data = new FormData(form)
        const new_password = formText(data, 'new_password')
        change.mutate(
          hasPassword
            ? {
                current_password: formText(data, 'current_password'),
                new_password,
              }
            : { new_password },
          {
            onSuccess: () => {
              form.reset()
            },
          },
        )
      }}
    >
      {hasPassword && (
        <Field
          label={t('settings.currentPassword')}
          name="current_password"
          type="password"
          autoComplete="current-password"
          required
        />
      )}
      <Field
        label={t('settings.newPassword')}
        hint={t('settings.newPasswordHint')}
        name="new_password"
        type="password"
        autoComplete="new-password"
        required
        minLength={8}
      />
      {change.error instanceof ApiError &&
      change.error.key === 'auth.invalidCredentials' ? (
        <p role="alert" className="text-sm text-destructive">
          {t('settings.currentPasswordWrong')}
        </p>
      ) : (
        <FormError error={change.error} />
      )}
      <div>
        <Button type="submit" disabled={change.isPending}>
          {t(
            hasPassword ? 'settings.changePassword' : 'settings.createPassword',
          )}
        </Button>
      </div>
    </form>
  )
}
