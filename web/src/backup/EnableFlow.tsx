import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { api, call } from '@/api/client'
import { ApiError } from '@/api/errors'
import type { BackupStatus } from '@/api/types'
import { Field, FormError, formText } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { KeyBox } from './KeyBox'
import { backupKey } from './status'

const COMPOSE_EXAMPLE = `services:
  app:
    volumes:
      - /volume1/backup/chagokmoa:/backup`

// 서버 판정은 "있고 쓰기 가능" 하나라 없음과 권한 문제를 함께 안내.
export function MountNotice() {
  const { t } = useTranslation()
  return (
    <div className="grid gap-2 rounded-xl bg-muted p-4 text-sm">
      <p className="font-semibold">{t('backup.mountTitle')}</p>
      <p>{t('backup.mountDesc')}</p>
      <pre className="overflow-x-auto rounded-md bg-background p-3 font-mono text-xs">
        {COMPOSE_EXAMPLE}
      </pre>
      <p className="text-muted-foreground">{t('backup.mountOwner')}</p>
    </div>
  )
}

// 경합에서 진 쪽(running, alreadyConfigured)과 폴더 상태 변화(409)는 새 상태 조회로 판단.
function useConflictRefresh(): (e: unknown) => void {
  const qc = useQueryClient()
  return (e) => {
    if (e instanceof ApiError && e.status === 409) {
      void qc.invalidateQueries({ queryKey: backupKey })
    }
  }
}

function useEnable() {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const onConflict = useConflictRefresh()
  return useMutation({
    gcTime: 0,
    mutationFn: (body: { key: string; confirmed?: boolean }) =>
      call(api.POST('/api/backup/enable', { body })),
    onSuccess: (status) => {
      qc.setQueryData(backupKey, status)
      toast(t('backup.enabled'))
    },
    onError: onConflict,
  })
}

// 다른 요청이 먼저 켠 경우. 새로 고침 뒤 설정 화면으로 바뀌므로 문구 없음.
function EnableError({ error }: { error: unknown }) {
  if (error instanceof ApiError && error.key === 'backup.alreadyConfigured') {
    return null
  }
  return <FormError error={error} />
}

function NewRepo() {
  const { t } = useTranslation()
  const [recoveryKey, setRecoveryKey] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const checkId = useId()
  const onConflict = useConflictRefresh()
  const create = useMutation({
    gcTime: 0,
    mutationFn: () => call(api.POST('/api/backup/key')),
    onSuccess: (res) => {
      setRecoveryKey(res.key)
    },
    onError: onConflict,
  })
  const enable = useEnable()

  if (recoveryKey === null) {
    return (
      <>
        <FormError error={create.error} />
        <Button
          className="justify-self-start"
          disabled={create.isPending}
          onClick={() => {
            create.mutate()
          }}
        >
          {t('backup.createKey')}
        </Button>
      </>
    )
  }
  return (
    <>
      <KeyBox recoveryKey={recoveryKey} />
      <div className="flex items-center gap-2">
        <Checkbox
          id={checkId}
          checked={saved}
          onCheckedChange={(v) => {
            setSaved(v === true)
          }}
        />
        <Label htmlFor={checkId}>{t('backup.keySaved')}</Label>
      </div>
      <EnableError error={enable.error} />
      <Button
        className="justify-self-start"
        disabled={!saved || enable.isPending}
        onClick={() => {
          enable.mutate({ key: recoveryKey, confirmed: true })
        }}
      >
        {t('backup.enable')}
      </Button>
    </>
  )
}

function ExistingRepo() {
  const { t } = useTranslation()
  const enable = useEnable()
  return (
    <form
      className="grid gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        // 붙여 넣을 때 딸려 온 앞뒤 공백 제거. 서버는 기존 저장소 키 공백 허용이라 화면에서.
        const key = formText(new FormData(e.currentTarget), 'key').trim()
        if (key) enable.mutate({ key })
      }}
    >
      <p className="text-sm">{t('backup.existingDesc')}</p>
      <p className="text-sm text-muted-foreground">
        {t('backup.existingRestore')}
      </p>
      <Field
        label={t('backup.existingKey')}
        name="key"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        className="font-mono"
        required
      />
      <EnableError error={enable.error} />
      <Button
        type="submit"
        className="justify-self-start"
        disabled={enable.isPending}
      >
        {t('backup.enable')}
      </Button>
    </form>
  )
}

// 키 저장 전. 폴더 연결 확인 후 폴더 내용으로 분기.
export function EnableFlow({ status }: { status: BackupStatus }) {
  const { t } = useTranslation()
  let body
  if (!status.mounted) {
    body = <MountNotice />
  } else if (status.repo_state === 'empty') {
    body = <NewRepo />
  } else if (status.repo_state === 'restic') {
    body = <ExistingRepo />
  } else {
    body = (
      <p role="alert" className="text-sm text-destructive">
        {status.repo_state === 'foreign'
          ? t('backup.foreign')
          : t('backup.unreadable')}
      </p>
    )
  }
  return (
    <section className="grid gap-3">
      <h2 className="text-base font-bold">{t('backup.setupTitle')}</h2>
      <p className="text-sm text-muted-foreground">{t('backup.setupDesc')}</p>
      {body}
    </section>
  )
}
