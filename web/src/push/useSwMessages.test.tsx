import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook } from '@testing-library/react'
import type { ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { notificationsKey } from '@/notifications/useUnreadCount'
import type { AppRouter } from '@/router'
import { useSwMessages } from './useSwMessages'

type Listener = (e: MessageEvent<unknown>) => void

// 서비스 워커 메시지 수신기만 흉내. 등록된 수신기로 직접 전달.
function setup() {
  const listeners = new Set<Listener>()
  vi.stubGlobal('navigator', {
    serviceWorker: {
      addEventListener: (_type: string, l: Listener) => listeners.add(l),
      removeEventListener: (_type: string, l: Listener) => listeners.delete(l),
    },
  })
  const qc = new QueryClient()
  const invalidate = vi.spyOn(qc, 'invalidateQueries')
  const navigate = vi.fn().mockResolvedValue(undefined)
  const router = { navigate } as unknown as AppRouter
  const { unmount } = renderHook(
    () => {
      useSwMessages(router)
    },
    {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    },
  )
  const send = (data: unknown) => {
    for (const l of listeners) l(new MessageEvent('message', { data }))
  }
  return { send, invalidate, navigate, listeners, unmount }
}

describe('useSwMessages', () => {
  it('push_invalidates_notifications', () => {
    const { send, invalidate, navigate } = setup()
    send({ type: 'cgm.push' })
    expect(invalidate).toHaveBeenCalledWith({ queryKey: notificationsKey })
    expect(navigate).not.toHaveBeenCalled()
  })

  it('navigate_routes_to_href', () => {
    const { send, invalidate, navigate } = setup()
    send({ type: 'cgm.navigate', url: '/notifications' })
    expect(navigate).toHaveBeenCalledWith({ href: '/notifications' })
    expect(invalidate).not.toHaveBeenCalled()
  })

  it('ignores_unknown_messages', () => {
    const { send, invalidate, navigate } = setup()
    send({ type: 'cgm.navigate', url: 1 })
    send(null)
    send('x')
    expect(navigate).not.toHaveBeenCalled()
    expect(invalidate).not.toHaveBeenCalled()
  })

  it('removes_listener_on_unmount', () => {
    const { listeners, unmount } = setup()
    expect(listeners.size).toBe(1)
    unmount()
    expect(listeners.size).toBe(0)
  })
})
