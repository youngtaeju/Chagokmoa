import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { notificationsKey } from '@/notifications/useUnreadCount'
import type { AppRouter } from '@/router'

// 서비스 워커 메시지: cgm.push 는 알림 목록 갱신, cgm.navigate 는 알림 클릭 이동.
export function useSwMessages(router: AppRouter): void {
  const qc = useQueryClient()
  useEffect(() => {
    if (!('serviceWorker' in navigator)) return
    const sw = navigator.serviceWorker
    const onMessage = (e: MessageEvent<unknown>) => {
      const d = e.data as { type?: string; url?: unknown } | null
      if (d?.type === 'cgm.push') {
        void qc.invalidateQueries({ queryKey: notificationsKey })
      } else if (d?.type === 'cgm.navigate' && typeof d.url === 'string') {
        void router.navigate({ href: d.url })
      }
    }
    sw.addEventListener('message', onMessage)
    return () => {
      sw.removeEventListener('message', onMessage)
    }
  }, [qc, router])
}
