import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useId } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { api, call } from '@/api/client'
import { ApiError } from '@/api/errors'
import type { BackupRun, BackupStatus, RepoReport } from '@/api/types'
import { FormError } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { useSettings } from '@/settings/useSettings'
import { cn } from '@/lib/utils'
import { formatMoment } from '@/viewer/zoned'
import { MountNotice } from './EnableFlow'
import {
  backupKey,
  durationText,
  errorText,
  sizeText,
  warningText,
} from './status'

function useMoment(): (iso: string) => string {
  const { t, i18n } = useTranslation()
  const tz =
    useSettings()?.timezone ?? Intl.DateTimeFormat().resolvedOptions().timeZone
  return (iso) =>
    formatMoment(iso, tz, i18n.language, {
      am: t('viewer.am'),
      pm: t('viewer.pm'),
    })
}

function RepoLine({ label, report }: { label: string; report: RepoReport }) {
  const { t, i18n } = useTranslation()
  return (
    <div className="grid gap-0.5 text-sm">
      <span className="font-semibold">{label}</span>
      {report.error ? (
        <span className="break-words text-destructive">{report.error}</span>
      ) : (
        <span className="text-muted-foreground tabular-nums">
          {t('backup.added', {
            count: report.files_new ?? 0,
            size: sizeText(report.data_added ?? 0, i18n.language),
          })}
          {report.duration_ms != null &&
            `, ${t('backup.took', { duration: durationText(t, report.duration_ms) })}`}
        </span>
      )}
      {report.warning &&
        warningText(t, report.warning).map((w) => <span key={w}>{w}</span>)}
    </div>
  )
}

function statusClass(run: BackupRun): string {
  return run.status === 'failed'
    ? 'text-destructive'
    : run.status === 'warning'
      ? 'text-accent-ink'
      : ''
}

// 시도하지 않은 저장소는 서버가 빈 객체({})로 저장.
function ran(r?: RepoReport | null): r is RepoReport {
  return r != null && (r.snapshot_id != null || r.error != null)
}

function LastRun({ run }: { run: BackupRun }) {
  const { t } = useTranslation()
  const moment = useMoment()
  // 저장소 결과에 없는 실패(덤프, 중단)만 따로.
  const repoFailed = Boolean(run.local?.error ?? run.remote?.error)
  return (
    <div className="grid gap-2">
      <p className="text-sm">
        <time dateTime={run.finished_at ?? run.started_at}>
          {moment(run.finished_at ?? run.started_at)}
        </time>{' '}
        <span className={cn('font-semibold', statusClass(run))}>
          {t(`backup.status.${run.status}`)}
        </span>
      </p>
      {ran(run.local) && (
        <RepoLine label={t('backup.local')} report={run.local} />
      )}
      {ran(run.remote) && (
        <RepoLine label={t('backup.remote')} report={run.remote} />
      )}
      {run.error && !repoFailed && (
        <p className="text-sm break-words text-destructive">
          {errorText(t, run.error)}
        </p>
      )}
    </div>
  )
}

function NextRun({ status }: { status: BackupStatus }) {
  const { t } = useTranslation()
  const moment = useMoment()
  // 미연결이면 예정 시각 대신 연결 안내.
  if (!status.mounted) return <MountNotice />
  const next = status.next_run
  const now = new Date()
  return (
    <div className="grid gap-1">
      <h3 className="text-sm font-semibold">{t('backup.nextRun')}</h3>
      <p className="text-sm">
        {!next ? (
          t('backup.autoOff')
        ) : Date.parse(next) <= now.getTime() ? (
          // 지난 슬롯은 일정 확인(1분 주기)이 곧 실행.
          t('backup.nextSoon')
        ) : (
          <time dateTime={next}>{moment(next)}</time>
        )}
      </p>
    </div>
  )
}

function Runs({ runs }: { runs: BackupRun[] }) {
  const { t } = useTranslation()
  const moment = useMoment()
  const titleId = useId()
  return (
    <div className="grid gap-1">
      <h3 id={titleId} className="text-sm font-semibold">
        {t('backup.runsTitle')}
      </h3>
      <ul aria-labelledby={titleId} className="grid gap-1.5">
        {runs.map((r) => (
          <li key={r.id} className="grid text-sm">
            <span className="flex flex-wrap gap-x-2">
              <time dateTime={r.started_at} className="tabular-nums">
                {moment(r.started_at)}
              </time>
              <span className="text-muted-foreground">
                {t(`backup.trigger.${r.trigger}`)}
              </span>
              <span className={statusClass(r)}>
                {t(`backup.status.${r.status}`)}
              </span>
            </span>
            {r.status === 'failed' && r.error && (
              <span className="break-words text-muted-foreground">
                {errorText(t, r.error)}
              </span>
            )}
          </li>
        ))}
      </ul>
    </div>
  )
}

export function StatusSection({ status }: { status: BackupStatus }) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const titleId = useId()
  const head = status.runs[0]
  const current = head?.status === 'running' ? head : null
  const runNow = useMutation({
    mutationFn: () => call(api.POST('/api/backup/run')),
    onSuccess: async () => {
      toast(t('backup.runStarted'))
      await qc.invalidateQueries({ queryKey: backupKey })
    },
    // 일정 실행이 먼저 시작했으면(backup.running) 새 상태가 진행 표시로 전환.
    onError: (e) => {
      if (e instanceof ApiError && e.status === 409) {
        void qc.invalidateQueries({ queryKey: backupKey })
      }
    },
  })
  const runError =
    runNow.error instanceof ApiError && runNow.error.key === 'backup.running'
      ? null
      : runNow.error

  return (
    <section aria-labelledby={titleId} className="grid gap-4">
      <h2 id={titleId} className="text-base font-bold">
        {t('backup.statusTitle')}
      </h2>
      {current && (
        <div role="status" className="grid gap-0.5 rounded-xl bg-muted p-3">
          <p className="text-sm font-semibold">{t('backup.running')}</p>
          {current.phase && (
            <p className="text-sm text-muted-foreground">
              {t(`backup.phase.${current.phase}`)}
            </p>
          )}
        </div>
      )}
      <div className="grid gap-1">
        <h3 className="text-sm font-semibold">{t('backup.lastRun')}</h3>
        {status.last_run ? (
          <LastRun run={status.last_run} />
        ) : (
          <p className="text-sm text-muted-foreground">{t('backup.never')}</p>
        )}
      </div>
      <NextRun status={status} />
      <FormError error={runError} />
      <Button
        // 첫 백업 전이면 주 버튼으로 권함.
        variant={status.runs.length === 0 ? 'default' : 'outline'}
        className="justify-self-start"
        disabled={!status.mounted || current !== null || runNow.isPending}
        onClick={() => {
          runNow.mutate()
        }}
      >
        {t('backup.runNow')}
      </Button>
      {status.runs.length > 0 && <Runs runs={status.runs} />}
    </section>
  )
}
