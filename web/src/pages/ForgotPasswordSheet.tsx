import { useMutation, useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { api, call } from '@/api/client'
import { authStatusQuery } from '@/auth/session'
import { Field, FormError, formText } from '@/components/Field'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from '@/components/ui/sheet'

// 메일 설정됨이면 재설정 링크 요청 폼, 아니면 관리자 요청 안내.
export function ForgotPasswordSheet() {
  const { t } = useTranslation()
  const mailEnabled = useQuery(authStatusQuery).data?.mail_enabled === true
  const send = useMutation({
    mutationFn: (login: string) =>
      call(api.POST('/api/auth/forgot-password', { body: { login } })),
  })
  return (
    <Sheet
      onOpenChange={(open) => {
        if (!open) send.reset()
      }}
    >
      <SheetTrigger asChild>
        <Button
          type="button"
          variant="link"
          className="min-h-11 px-3 text-[15px] font-semibold underline"
        >
          {t('auth.login.forgot')}
        </Button>
      </SheetTrigger>
      <SheetContent side="bottom" className="mx-auto max-w-[400px]">
        <SheetHeader className="pb-6">
          <SheetTitle>{t('auth.login.forgotTitle')}</SheetTitle>
          <SheetDescription
            className={mailEnabled && !send.isSuccess ? 'sr-only' : undefined}
          >
            {!mailEnabled
              ? t('auth.login.forgotAskAdmin')
              : send.isSuccess
                ? t('auth.login.forgotDone')
                : t('auth.login.forgotTitle')}
          </SheetDescription>
        </SheetHeader>
        {mailEnabled && !send.isSuccess && (
          <form
            className="grid gap-4 px-4 pb-6"
            onSubmit={(e) => {
              e.preventDefault()
              send.mutate(
                formText(new FormData(e.currentTarget), 'login').trim(),
              )
            }}
          >
            <Field
              label={t('auth.login.forgotLogin')}
              name="login"
              autoComplete="username"
              autoCapitalize="none"
              required
            />
            <FormError error={send.error} />
            <Button type="submit" disabled={send.isPending}>
              {t('auth.login.forgotSend')}
            </Button>
          </form>
        )}
      </SheetContent>
    </Sheet>
  )
}
