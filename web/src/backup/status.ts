import { useQuery } from '@tanstack/react-query'
import type { TFunction } from 'i18next'
import { api, call } from '@/api/client'

export const backupKey = ['backup'] as const
export const BACKUP_POLL_MS = 2000

// 실행 중이면 주기 조회. 진행 중 실행은 runs 첫 항목.
export function useBackupStatus() {
  return useQuery({
    queryKey: backupKey,
    queryFn: () => call(api.GET('/api/backup')),
    refetchInterval: (q) =>
      q.state.data?.runs[0]?.status === 'running' ? BACKUP_POLL_MS : false,
  })
}

// 서버 고정 영어 문구(run.rs, restic.rs) → 번역. 모르는 문구는 원문.
const VANISHED = /^(\d+) files vanished during backup$/

export function warningText(t: TFunction, warning: string): string[] {
  return warning.split('; ').map((w) => {
    if (w === 'local repository created') return t('backup.warnLocalCreated')
    const m = VANISHED.exec(w)
    if (m) return t('backup.warnVanished', { count: Number(m[1]) })
    return w
  })
}

export function errorText(t: TFunction, error: string): string {
  return error === 'interrupted' ? t('backup.interrupted') : error
}

export function durationText(t: TFunction, ms: number): string {
  const sec = Math.max(1, Math.round(ms / 1000))
  if (sec < 60) return t('backup.seconds', { count: sec })
  const min = Math.round(sec / 60)
  if (min < 60) return t('backup.minutes', { count: min })
  return t('backup.hours', { h: Math.floor(min / 60), m: min % 60 })
}

export function sizeText(bytes: number, lng: string): string {
  const units = [
    ['gigabyte', 1e9],
    ['megabyte', 1e6],
    ['kilobyte', 1e3],
  ] as const
  const [unit, div] = units.find(([, d]) => bytes >= d) ?? ['byte', 1]
  return new Intl.NumberFormat(lng, {
    style: 'unit',
    unit,
    maximumFractionDigits: 1,
  }).format(bytes / div)
}
