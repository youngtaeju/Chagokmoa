import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { api, call } from '@/api/client'
import { FormError } from '@/components/Field'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

const regenKey = ['derivatives', 'regenerate'] as const
export const REGEN_POLL_MS = 3000

// 파생물(썸네일, 재생용 영상) 재생성. 남은 수는 0 이 될 때까지 주기 조회.
export function RegenSection() {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const [started, setStarted] = useState(false)

  const status = useQuery({
    queryKey: regenKey,
    queryFn: () => call(api.GET('/api/derivatives/regenerate')),
    refetchInterval: (q) =>
      (q.state.data?.remaining ?? 0) > 0 ? REGEN_POLL_MS : false,
  })
  const start = useMutation({
    mutationFn: (missingOnly: boolean) =>
      call(
        api.POST('/api/derivatives/regenerate', {
          body: { missing_only: missingOnly },
        }),
      ),
    onSuccess: (res) => {
      qc.setQueryData(regenKey, { remaining: res.queued, failed: 0 })
      setStarted(true)
      setConfirming(false)
    },
  })

  const remaining = status.data?.remaining ?? 0
  const failed = status.data?.failed ?? 0
  const running = remaining > 0

  function close() {
    setConfirming(false)
    start.reset()
  }

  return (
    <section className="grid gap-3 border-t border-border pt-4">
      <h2 className="text-base font-bold">{t('backup.regenTitle')}</h2>
      <p className="text-sm text-muted-foreground">{t('backup.regenDesc')}</p>
      <FormError error={status.error} />
      {running && (
        <p role="status" className="text-sm tabular-nums">
          {t('backup.regenRemaining', { count: remaining })}
        </p>
      )}
      {!running && started && failed === 0 && (
        <p role="status" className="text-sm">
          {t('backup.regenDone')}
        </p>
      )}
      {!running && started && failed > 0 && (
        <p role="status" className="text-sm text-destructive">
          {t('backup.regenFailed', { count: failed })}
        </p>
      )}
      <Button
        variant="outline"
        className="justify-self-start"
        disabled={running}
        onClick={() => {
          setConfirming(true)
        }}
      >
        {t('backup.regenButton')}
      </Button>
      <Dialog
        open={confirming}
        onOpenChange={(open) => {
          if (!open) close()
        }}
      >
        <DialogContent role="alertdialog">
          <DialogHeader>
            <DialogTitle>{t('backup.regenConfirmTitle')}</DialogTitle>
            <DialogDescription>{t('backup.regenConfirm')}</DialogDescription>
          </DialogHeader>
          <FormError error={start.error} />
          <DialogFooter>
            <Button variant="outline" onClick={close}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="outline"
              disabled={start.isPending}
              onClick={() => {
                start.mutate(true)
              }}
            >
              {t('backup.regenMissing')}
            </Button>
            <Button
              disabled={start.isPending}
              onClick={() => {
                start.mutate(false)
              }}
            >
              {t('backup.regenAll')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  )
}
