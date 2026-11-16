import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useState, useSyncExternalStore } from 'react'
import { meQuery } from '@/auth/session'
import {
  pushSupport,
  reconcileDevice,
  subscribeDevice,
  unsubscribeDevice,
  type PushSupport,
} from './device'

// 구독 확인이 필요한 상태. 그 밖은 브라우저 구독 없음.
function canHaveSubscription(s: PushSupport): boolean {
  return s === 'supported' || s === 'denied'
}

export function usePushDevice(): {
  support: PushSupport
  // null = 확인 중.
  on: boolean | null
  pending: boolean
  enable: () => Promise<void>
  disable: () => Promise<void>
} {
  const [support, setSupport] = useState(pushSupport)
  const [on, setOn] = useState<boolean | null>(() =>
    canHaveSubscription(support) ? null : false,
  )
  const [pending, setPending] = useState(false)

  useEffect(() => {
    if (!canHaveSubscription(support)) return
    let cancelled = false
    void reconcileDevice()
      .catch(() => false)
      .then((v) => {
        if (!cancelled) setOn(v)
      })
    return () => {
      cancelled = true
    }
  }, [support])

  const run = useCallback(async (fn: () => Promise<void>, next: boolean) => {
    setPending(true)
    try {
      await fn()
      setOn(next)
    } finally {
      setPending(false)
      // 권한 거부 등 변화 반영.
      setSupport(pushSupport())
    }
  }, [])

  return {
    support,
    on,
    pending,
    enable: () => run(subscribeDevice, true),
    disable: () => run(unsubscribeDevice, false),
  }
}

// me 캐시의 사용자 id. qc.clear() 뒤 새로 만든 me 쿼리도 따라가도록 캐시 전체 구독.
function useSignedInUserId(): string | undefined {
  const qc = useQueryClient()
  return useSyncExternalStore(
    (onChange) => qc.getQueryCache().subscribe(onChange),
    () => qc.getQueryData(meQuery.queryKey)?.id,
  )
}

// 로그인할 때마다(같은 탭 재로그인 포함) 한 번. 이전 계정의 구독이 브라우저에 남아 알림이 새는 것 방지.
export function useReconcilePushDevice(): void {
  const userId = useSignedInUserId()
  useEffect(() => {
    if (userId === undefined) return
    if (!canHaveSubscription(pushSupport())) return
    reconcileDevice().catch(() => undefined)
  }, [userId])
}
