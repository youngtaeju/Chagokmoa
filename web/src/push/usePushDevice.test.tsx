import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { describe, expect, it } from 'vitest'
import { meQueryKey, setSignedIn, useLogout } from '@/auth/session'
import { json, mockFetch } from '@/test/fetch'
import { userFixture } from '@/test/fixtures'
import { fakeSub, installPush } from '@/test/push'
import { createTestQueryClient } from '@/test/query'
import { usePushDevice, useReconcilePushDevice } from './usePushDevice'

function wrapperFor(qc: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  )
}

const wrapper = wrapperFor(new QueryClient())

describe('usePushDevice', () => {
  it('on_when_endpoints_match', async () => {
    installPush({ current: fakeSub('https://fcm.googleapis.com/a') })
    mockFetch({
      'GET /api/push/subscription': () =>
        json(200, { endpoint: 'https://fcm.googleapis.com/a' }),
    })
    const { result } = renderHook(() => usePushDevice(), { wrapper })
    await waitFor(() => {
      expect(result.current.on).toBe(true)
    })
  })

  it('on_is_null_while_checking', async () => {
    installPush({ current: fakeSub('https://fcm.googleapis.com/a') })
    let release: () => void = () => undefined
    mockFetch({
      'GET /api/push/subscription': () =>
        new Promise((resolve) => {
          release = () => {
            resolve(json(200, { endpoint: 'https://fcm.googleapis.com/a' }))
          }
        }),
    })
    const { result } = renderHook(() => usePushDevice(), { wrapper })
    await act(() => new Promise((r) => setTimeout(r, 0)))
    expect(result.current.on).toBeNull()
    release()
    await waitFor(() => {
      expect(result.current.on).toBe(true)
    })
  })

  it('off_without_check_when_unsupported', () => {
    installPush({ current: null })
    Reflect.deleteProperty(window, 'PushManager')
    const { result } = renderHook(() => usePushDevice(), { wrapper })
    expect(result.current.on).toBe(false)
  })

  it('unsubscribes_browser_when_endpoints_differ', async () => {
    const sub = fakeSub('https://fcm.googleapis.com/a')
    installPush({ current: sub })
    mockFetch({
      'GET /api/push/subscription': () =>
        json(200, { endpoint: 'https://fcm.googleapis.com/other' }),
    })
    const { result } = renderHook(() => usePushDevice(), { wrapper })
    await waitFor(() => {
      expect(sub.unsubscribe).toHaveBeenCalled()
    })
    expect(result.current.on).toBe(false)
  })

  it('enable_and_disable', async () => {
    const sub = fakeSub('https://fcm.googleapis.com/new')
    const { pushManager } = installPush({ current: null, subscribeResult: sub })
    mockFetch({
      'GET /api/push/key': () => json(200, { public_key: 'AQID' }),
      'PUT /api/push/subscription': () => json(204, null),
      'DELETE /api/push/subscription': () => json(204, null),
    })
    const { result } = renderHook(() => usePushDevice(), { wrapper })
    await act(async () => {
      await result.current.enable()
    })
    expect(result.current.on).toBe(true)
    pushManager.getSubscription.mockResolvedValue(sub)
    await act(async () => {
      await result.current.disable()
    })
    expect(sub.unsubscribe).toHaveBeenCalled()
    expect(result.current.on).toBe(false)
  })
})

describe('useReconcilePushDevice', () => {
  it('unsubscribes_stale_browser_subscription_when_signed_in', async () => {
    const sub = fakeSub('https://fcm.googleapis.com/a')
    installPush({ current: sub })
    mockFetch({
      'GET /api/push/subscription': () => json(200, { endpoint: null }),
    })
    const qc = new QueryClient()
    qc.setQueryData(meQueryKey, userFixture('guardian'))
    renderHook(
      () => {
        useReconcilePushDevice()
      },
      { wrapper: wrapperFor(qc) },
    )
    await waitFor(() => {
      expect(sub.unsubscribe).toHaveBeenCalled()
    })
  })

  it('does_nothing_when_signed_out', async () => {
    const sub = fakeSub('https://fcm.googleapis.com/a')
    const { pushManager } = installPush({ current: sub })
    const qc = new QueryClient()
    qc.setQueryData(meQueryKey, null)
    renderHook(
      () => {
        useReconcilePushDevice()
      },
      { wrapper: wrapperFor(qc) },
    )
    await act(() => new Promise((r) => setTimeout(r, 20)))
    expect(pushManager.getSubscription).not.toHaveBeenCalled()
    expect(sub.unsubscribe).not.toHaveBeenCalled()
  })

  it('does_nothing_without_browser_subscription', async () => {
    const { pushManager } = installPush({ current: null })
    const qc = new QueryClient()
    qc.setQueryData(meQueryKey, userFixture('guardian'))
    renderHook(
      () => {
        useReconcilePushDevice()
      },
      { wrapper: wrapperFor(qc) },
    )
    await waitFor(() => {
      expect(pushManager.getSubscription).toHaveBeenCalled()
    })
  })

  // 로그아웃의 qc.clear() 뒤에도 me 를 따라가 다른 사용자 로그인 때 다시 확인.
  it('rechecks_after_logout_and_login_in_same_tab', async () => {
    const sub = fakeSub('https://fcm.googleapis.com/a')
    installPush({ current: sub })
    let gets = 0
    mockFetch({
      'GET /api/push/subscription': () => {
        gets += 1
        return json(200, { endpoint: 'https://fcm.googleapis.com/a' })
      },
      'POST /api/auth/logout': () => new Response(null, { status: 204 }),
    })
    const qc = createTestQueryClient()
    qc.setQueryData(meQueryKey, userFixture('guardian'))
    // 실제 앱처럼 로그아웃 화면과 별개 컴포넌트. 로그아웃이 이 훅을 다시 렌더하지 않음.
    renderHook(
      () => {
        useReconcilePushDevice()
      },
      { wrapper: wrapperFor(qc) },
    )
    const { result } = renderHook(() => useLogout(), {
      wrapper: wrapperFor(qc),
    })
    await waitFor(() => {
      expect(gets).toBe(1)
    })
    await act(async () => {
      await result.current()
    })
    expect(qc.getQueryData(meQueryKey)).toBeNull()
    act(() => {
      setSignedIn(qc, { ...userFixture('family'), id: 'other-user' })
    })
    await waitFor(() => {
      expect(gets).toBe(2)
    })
  })
})
