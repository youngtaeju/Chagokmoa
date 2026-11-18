import { useMutation } from '@tanstack/react-query'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { api, call } from '@/api/client'
import { ApiError } from '@/api/errors'
import { FormError, formText } from '@/components/Field'
import { PasswordField } from '@/components/PasswordField'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { KeyBox } from './KeyBox'

// 복구 키 다시 보기. 소유자 비밀번호 재확인 후.
export function KeySection() {
  const { t } = useTranslation()
  const titleId = useId()
  const [open, setOpen] = useState(false)
  const reveal = useMutation({
    gcTime: 0,
    mutationFn: (password: string) =>
      call(api.POST('/api/backup/key/reveal', { body: { password } })),
  })

  function close() {
    setOpen(false)
    // 닫으면 키를 화면에서 지움.
    reveal.reset()
  }

  return (
    <section
      aria-labelledby={titleId}
      className="grid gap-3 border-t border-border pt-4"
    >
      <h2 id={titleId} className="text-base font-bold">
        {t('backup.keyTitle')}
      </h2>
      <p className="text-sm text-muted-foreground">{t('backup.keyDesc')}</p>
      <Button
        variant="outline"
        className="justify-self-start"
        onClick={() => {
          setOpen(true)
        }}
      >
        {t('backup.keyReveal')}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          if (!next) close()
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('backup.keyReveal')}</DialogTitle>
            <DialogDescription>{t('backup.keyDesc')}</DialogDescription>
          </DialogHeader>
          {reveal.data ? (
            <KeyBox recoveryKey={reveal.data.key} />
          ) : (
            <form
              className="grid gap-3"
              onSubmit={(e) => {
                e.preventDefault()
                reveal.mutate(
                  formText(new FormData(e.currentTarget), 'password'),
                )
              }}
            >
              <PasswordField
                label={t('backup.password')}
                name="password"
                autoComplete="current-password"
                required
              />
              {reveal.error instanceof ApiError &&
              reveal.error.key === 'auth.invalidCredentials' ? (
                <p role="alert" className="text-sm text-destructive">
                  {t('backup.revealWrong')}
                </p>
              ) : (
                <FormError error={reveal.error} />
              )}
              <Button
                type="submit"
                className="justify-self-start"
                disabled={reveal.isPending}
              >
                {t('backup.keyShow')}
              </Button>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </section>
  )
}
