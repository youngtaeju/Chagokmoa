import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
} from '@tanstack/react-query'
import { useMemo } from 'react'
import { api, call } from '@/api/client'
import { notificationsKey, unreadKey } from './useUnreadCount'

const listKey = [...notificationsKey, 'list'] as const

// 최신순 목록. seenAt: 이번 화면 진입 뒤 받은 첫 페이지의 seen_at(이전 방문 캐시 제외).
export function useNotifications() {
  const query = useInfiniteQuery({
    queryKey: listKey,
    queryFn: ({ pageParam }) =>
      call(
        api.GET('/api/notifications', {
          params: { query: { cursor: pageParam } },
        }),
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (p) => p.next_cursor ?? undefined,
  })
  const { data } = query
  const items = useMemo(() => data?.pages.flatMap((p) => p.items) ?? [], [data])
  return {
    items,
    seenAt: query.isFetchedAfterMount ? data?.pages[0]?.seen_at : undefined,
    fetchNextPage: query.fetchNextPage,
    hasNextPage: query.hasNextPage,
    isFetchingNextPage: query.isFetchingNextPage,
    isFetchNextPageError: query.isFetchNextPageError,
    isLoadingError: query.isLoadingError,
    status: query.status,
    error: query.error,
    refetch: query.refetch,
  }
}

// 본 시각 기록. 성공 즉시 종 배지 0(진행 중 조회가 덮어쓰지 않게 먼저 취소).
export function useMarkSeen() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: () => call(api.POST('/api/notifications/seen')),
    onSuccess: async () => {
      await qc.cancelQueries({ queryKey: unreadKey })
      qc.setQueryData(unreadKey, { count: 0 })
    },
  }).mutate
}
