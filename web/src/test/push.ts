import { vi } from 'vitest'

export interface FakeSub {
  endpoint: string
  unsubscribe: ReturnType<typeof vi.fn>
  toJSON: () => { endpoint: string; keys: { p256dh: string; auth: string } }
}

export function fakeSub(endpoint: string): FakeSub {
  return {
    endpoint,
    unsubscribe: vi.fn().mockResolvedValue(true),
    toJSON: () => ({ endpoint, keys: { p256dh: 'P', auth: 'A' } }),
  }
}

// 푸시 가능한 브라우저 환경 흉내. current 가 현재 브라우저 구독.
export function installPush(opts: {
  current: FakeSub | null
  permission?: NotificationPermission
  subscribeResult?: FakeSub
}) {
  const subscribe = vi
    .fn()
    .mockResolvedValue(
      opts.subscribeResult ?? fakeSub('https://fcm.googleapis.com/new'),
    )
  const pushManager = {
    getSubscription: vi.fn().mockResolvedValue(opts.current),
    subscribe,
  }
  const reg = { pushManager }
  const getRegistration = vi.fn().mockResolvedValue(reg)
  vi.stubGlobal('navigator', {
    userAgent: 'Mozilla/5.0 (X11; Linux)',
    maxTouchPoints: 0,
    serviceWorker: {
      getRegistration,
      ready: Promise.resolve(reg),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    },
  })
  vi.stubGlobal('PushManager', {})
  vi.stubGlobal('Notification', {
    permission: opts.permission ?? 'default',
    requestPermission: vi.fn().mockResolvedValue('granted'),
  })
  vi.stubGlobal('isSecureContext', true)
  return { subscribe, pushManager, getRegistration }
}
