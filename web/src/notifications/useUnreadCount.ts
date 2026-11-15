import { useQuery } from '@tanstack/react-query'
import { api, call } from '@/api/client'

export const notificationsKey = ['notifications'] as const
export const unreadKey = ['notifications', 'unread'] as const

// 앱이 보이는 동안 60초마다, 창 포커스 때 재조회.
export function useUnreadCount(): number | undefined {
  return useQuery({
    queryKey: unreadKey,
    queryFn: () => call(api.GET('/api/notifications/unread-count')),
    refetchInterval: 60_000,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
  }).data?.count
}

// 0 은 배지 없음, 100 이상은 99+ (서버는 100 에서 멈춤).
export function formatBadge(count: number): string | null {
  if (count <= 0) return null
  return count >= 100 ? '99+' : String(count)
}
