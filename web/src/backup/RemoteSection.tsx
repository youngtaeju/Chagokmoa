import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { api, call } from '@/api/client'
import type { BackupStatus } from '@/api/types'
import type { components } from '@/api/schema'
import { Field, FormError } from '@/components/Field'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { backupKey } from './status'

type RemoteRequest = components['schemas']['RemoteRequest']
type TestResult = components['schemas']['RemoteTestResult']

// 서버 허용 목록(routes.rs REMOTE_BACKENDS)과 같음. secret 은 가림 입력.
const BACKENDS: Record<string, { name: string; secret?: boolean }[]> = {
  s3: [
    { name: 'AWS_ACCESS_KEY_ID' },
    { name: 'AWS_SECRET_ACCESS_KEY', secret: true },
    { name: 'AWS_DEFAULT_REGION' },
    { name: 'AWS_SESSION_TOKEN', secret: true },
  ],
  b2: [{ name: 'B2_ACCOUNT_ID' }, { name: 'B2_ACCOUNT_KEY', secret: true }],
  // 인증값은 주소 안.
  rest: [],
  azure: [
    { name: 'AZURE_ACCOUNT_NAME' },
    { name: 'AZURE_ACCOUNT_KEY', secret: true },
    { name: 'AZURE_ACCOUNT_SAS', secret: true },
    { name: 'AZURE_ENDPOINT_SUFFIX' },
  ],
  gs: [
    { name: 'GOOGLE_PROJECT_ID' },
    { name: 'GOOGLE_APPLICATION_CREDENTIALS' },
  ],
}

function backendOf(repository: string): string | null {
  const prefix = repository.trim().split(':', 1)[0] ?? ''
  return prefix in BACKENDS ? prefix : null
}

function TestMessage({ result }: { result: TestResult }) {
  const { t } = useTranslation()
  if (result.ok) {
    return (
      <p role="status" className="text-sm">
        {result.state === 'empty'
          ? t('backup.remoteTestEmpty')
          : t('backup.remoteTestRestic')}
      </p>
    )
  }
  return (
    <div role="alert" className="grid gap-1 text-sm text-destructive">
      <p>
        {result.state === 'wrong_key'
          ? t('backup.remoteTestWrongKey')
          : result.state === 'foreign'
            ? t('backup.remoteTestForeign')
            : t('backup.remoteTestFailed')}
      </p>
      {result.message && (
        <pre className="overflow-x-auto rounded-md bg-muted p-2 text-xs whitespace-pre-wrap text-foreground">
          {result.message}
        </pre>
      )}
    </div>
  )
}

// 원격 입력. 저장은 지금 값 그대로 시험을 통과한 뒤에만. 저장된 값은 응답에 없어 항상 새로 입력.
function RemoteForm({
  replacing,
  onDone,
}: {
  replacing: boolean
  onDone: () => void
}) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const [repository, setRepository] = useState('')
  const [env, setEnv] = useState<Record<string, string>>({})
  const backend = backendOf(repository)
  const fields = backend ? (BACKENDS[backend] ?? []) : []
  const request: RemoteRequest = {
    repository: repository.trim(),
    env: Object.fromEntries(
      fields
        .map((f) => [f.name, env[f.name]?.trim() ?? ''] as const)
        .filter(([, v]) => v !== ''),
    ),
  }
  const requestKey = JSON.stringify(request)

  const test = useMutation({
    gcTime: 0,
    mutationFn: (body: RemoteRequest) =>
      call(api.POST('/api/backup/remote/test', { body })),
  })
  const save = useMutation({
    gcTime: 0,
    mutationFn: (body: RemoteRequest) =>
      call(api.PUT('/api/backup/remote', { body })),
    onSuccess: (s) => {
      qc.setQueryData(backupKey, s)
      toast(t('backup.remoteSaved'))
      onDone()
    },
  })
  // 시험한 값과 지금 값이 같을 때만 결과 표시와 저장.
  const tested =
    test.data && JSON.stringify(test.variables) === requestKey
      ? test.data
      : null

  return (
    <form
      className="grid gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        if (tested?.ok) save.mutate(request)
      }}
    >
      {replacing && (
        <p className="text-sm text-muted-foreground">
          {t('backup.remoteReenter')}
        </p>
      )}
      <Field
        label={t('backup.remoteRepository')}
        hint={t('backup.remoteRepositoryHint')}
        value={repository}
        onChange={(e) => {
          setRepository(e.target.value)
        }}
        placeholder="s3:https://s3.amazonaws.com/bucket/chagokmoa"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        required
      />
      {backend === 'rest' && (
        <p className="text-sm text-muted-foreground">
          {t('backup.remoteRestHint')}
        </p>
      )}
      {fields.map((f) => (
        <Field
          key={f.name}
          label={f.name}
          type={f.secret ? 'password' : 'text'}
          value={env[f.name] ?? ''}
          onChange={(e) => {
            const v = e.target.value
            setEnv((cur) => ({ ...cur, [f.name]: v }))
          }}
          // 로그인 비밀번호 자동 채움 방지.
          autoComplete={f.secret ? 'new-password' : 'off'}
          autoCapitalize="none"
          spellCheck={false}
        />
      ))}
      {fields.length > 0 && (
        <p className="text-xs text-muted-foreground">
          {backend === 'gs' && `${t('backup.remoteGsHint')} `}
          {t('backup.remoteEnvHint')}
        </p>
      )}
      {test.isPending && (
        <p role="status" className="text-sm text-muted-foreground">
          {t('backup.remoteTesting')}
        </p>
      )}
      {!test.isPending && tested && <TestMessage result={tested} />}
      {!test.isPending && JSON.stringify(test.variables) === requestKey && (
        <FormError error={test.error} />
      )}
      <FormError error={save.error} />
      {!tested?.ok && (
        <p className="text-xs text-muted-foreground">
          {t('backup.remoteSaveHint')}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="outline"
          disabled={test.isPending || save.isPending || !request.repository}
          onClick={() => {
            test.mutate(request)
          }}
        >
          {t('backup.remoteTest')}
        </Button>
        <Button type="submit" disabled={!tested?.ok || save.isPending}>
          {t('common.save')}
        </Button>
        <Button
          type="button"
          variant="ghost"
          disabled={save.isPending}
          onClick={onDone}
        >
          {t('common.cancel')}
        </Button>
      </div>
    </form>
  )
}

export function RemoteSection({ status }: { status: BackupStatus }) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const titleId = useId()
  const [editing, setEditing] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const remove = useMutation({
    mutationFn: () => call(api.DELETE('/api/backup/remote')),
    onSuccess: async () => {
      setConfirming(false)
      toast(t('backup.remoteDeleted'))
      await qc.invalidateQueries({ queryKey: backupKey })
    },
  })
  const remote = status.remote

  function closeConfirm() {
    setConfirming(false)
    remove.reset()
  }

  return (
    <section
      aria-labelledby={titleId}
      className="grid gap-3 border-t border-border pt-4"
    >
      <h2 id={titleId} className="text-base font-bold">
        {t('backup.remoteTitle')}
      </h2>
      <p className="text-sm text-muted-foreground">{t('backup.remoteDesc')}</p>
      {editing ? (
        <RemoteForm
          replacing={remote != null}
          onDone={() => {
            setEditing(false)
          }}
        />
      ) : remote ? (
        <>
          <div className="grid gap-1 rounded-xl bg-muted p-3 text-sm">
            <span className="font-mono break-all">{remote.repository}</span>
            {/* rest: 는 인증값이 주소 안이라 configured 대신 가린 비밀번호로 판단. */}
            {remote.configured ? (
              <span className="text-muted-foreground">
                {t('backup.remoteCreds')}
              </span>
            ) : (
              remote.repository.includes(':***@') && (
                <span className="text-muted-foreground">
                  {t('backup.remoteUrlCreds')}
                </span>
              )
            )}
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              variant="outline"
              onClick={() => {
                setEditing(true)
              }}
            >
              {t('backup.remoteChange')}
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                setConfirming(true)
              }}
            >
              {t('backup.remoteDelete')}
            </Button>
          </div>
        </>
      ) : (
        <>
          <p className="text-sm">{t('backup.remoteNone')}</p>
          <Button
            variant="outline"
            className="justify-self-start"
            onClick={() => {
              setEditing(true)
            }}
          >
            {t('backup.remoteAdd')}
          </Button>
        </>
      )}
      <Dialog
        open={confirming}
        onOpenChange={(open) => {
          if (!open) closeConfirm()
        }}
      >
        <DialogContent role="alertdialog">
          <DialogHeader>
            <DialogTitle>{t('backup.remoteDeleteTitle')}</DialogTitle>
            <DialogDescription>
              {t('backup.remoteDeleteDesc')}
            </DialogDescription>
          </DialogHeader>
          <FormError error={remove.error} />
          <DialogFooter>
            <Button variant="outline" onClick={closeConfirm}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => {
                remove.mutate()
              }}
            >
              {t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  )
}
