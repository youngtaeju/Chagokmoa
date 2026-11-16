import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api, call } from '@/api/client'
import type { components } from '@/api/schema'

export type PushPrefs = components['schemas']['PushPrefs']
export type PushKind = keyof PushPrefs

const prefsKey = ['push', 'prefs'] as const

export function usePushPrefs() {
  return useQuery({
    queryKey: prefsKey,
    queryFn: () => call(api.GET('/api/push/prefs')),
  })
}

// 낙관적 갱신. 실패 시 이 변경의 종류만 즉시 되돌림(다른 진행 중 변경 유지).
// 연속 조작 중 앞 응답이 뒤 변경을 덮지 않게 마지막 변경이 끝난 뒤 서버 값 다시 조회.
export function useSetPushPref() {
  const qc = useQueryClient()
  return useMutation({
    mutationKey: prefsKey,
    mutationFn: (patch: Partial<PushPrefs>) =>
      call(api.PATCH('/api/push/prefs', { body: patch })),
    onMutate: async (patch) => {
      await qc.cancelQueries({ queryKey: prefsKey })
      const prev = qc.getQueryData<PushPrefs>(prefsKey)
      if (!prev) return {}
      qc.setQueryData<PushPrefs>(prefsKey, { ...prev, ...patch })
      const revert: Partial<PushPrefs> = {}
      for (const k of Object.keys(patch) as PushKind[]) revert[k] = prev[k]
      return { revert }
    },
    onError: (_e, _p, ctx) => {
      const revert = ctx?.revert
      if (revert) {
        qc.setQueryData<PushPrefs>(prefsKey, (cur) =>
          cur ? { ...cur, ...revert } : cur,
        )
      }
    },
    // 자기 자신은 아직 진행 중으로 집계.
    onSettled: () =>
      qc.isMutating({ mutationKey: prefsKey }) === 1
        ? qc.invalidateQueries({ queryKey: prefsKey })
        : undefined,
  })
}
