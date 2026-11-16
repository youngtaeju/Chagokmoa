import { describe, expect, it, vi } from 'vitest'
import { json, mockFetch } from '@/test/fetch'
import { fakeSub, installPush } from '@/test/push'
import {
  pushSupport,
  reconcileDevice,
  subscribeDevice,
  unsubscribeBrowserOnly,
} from './device'

describe('pushSupport', () => {
  it('supported', () => {
    installPush({ current: null })
    expect(pushSupport()).toBe('supported')
  })

  it('denied', () => {
    installPush({ current: null, permission: 'denied' })
    expect(pushSupport()).toBe('denied')
  })

  it('unsupported_without_push_manager', () => {
    installPush({ current: null })
    Reflect.deleteProperty(window, 'PushManager')
    expect(pushSupport()).toBe('unsupported')
  })

  it('ios_install_when_not_standalone', () => {
    installPush({ current: null })
    vi.stubGlobal('navigator', {
      userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)',
      maxTouchPoints: 5,
    })
    expect(pushSupport()).toBe('ios-install')
  })
})

describe('subscribeDevice', () => {
  it('fetches_key_subscribes_then_puts', async () => {
    const order: string[] = []
    const { subscribe } = installPush({ current: null })
    subscribe.mockImplementation(() => {
      order.push('subscribe')
      return Promise.resolve({
        endpoint: 'https://fcm.googleapis.com/x',
        unsubscribe: vi.fn(),
        toJSON: () => ({
          endpoint: 'https://fcm.googleapis.com/x',
          keys: { p256dh: 'P', auth: 'A' },
        }),
      })
    })
    let body: unknown
    mockFetch({
      'GET /api/push/key': () => {
        order.push('key')
        return json(200, { public_key: 'AQID' })
      },
      'PUT /api/push/subscription': async (req) => {
        order.push('put')
        body = await req.json()
        return json(204, null)
      },
    })
    await subscribeDevice()
    expect(order).toEqual(['key', 'subscribe', 'put'])
    expect(body).toEqual({
      endpoint: 'https://fcm.googleapis.com/x',
      p256dh: 'P',
      auth: 'A',
    })
    const arg = subscribe.mock.calls[0]?.[0] as {
      userVisibleOnly: boolean
      applicationServerKey: Uint8Array
    }
    expect(arg.userVisibleOnly).toBe(true)
    expect(Array.from(arg.applicationServerKey)).toEqual([1, 2, 3])
  })

  it('waits_for_running_reconcile_before_subscribing', async () => {
    const order: string[] = []
    const { subscribe } = installPush({
      current: fakeSub('https://fcm.googleapis.com/a'),
    })
    subscribe.mockImplementation(() => {
      order.push('subscribe')
      return Promise.resolve(fakeSub('https://fcm.googleapis.com/a'))
    })
    let release: () => void = () => undefined
    mockFetch({
      'GET /api/push/subscription': () =>
        new Promise((resolve) => {
          release = () => {
            order.push('reconciled')
            resolve(json(200, { endpoint: 'https://fcm.googleapis.com/a' }))
          }
        }),
      'GET /api/push/key': () => json(200, { public_key: 'AQID' }),
      'PUT /api/push/subscription': () => json(204, null),
    })
    const reconciled = reconcileDevice()
    const subscribed = subscribeDevice()
    await new Promise((r) => setTimeout(r, 20))
    expect(subscribe).not.toHaveBeenCalled()
    release()
    await Promise.all([reconciled, subscribed])
    expect(order).toEqual(['reconciled', 'subscribe'])
  })

  it('reconcile_waits_for_running_subscribe', async () => {
    const order: string[] = []
    const sub = fakeSub('https://fcm.googleapis.com/new')
    const { subscribe, pushManager } = installPush({ current: null })
    subscribe.mockImplementation(() => {
      pushManager.getSubscription.mockResolvedValue(sub)
      return Promise.resolve(sub)
    })
    let registered = false
    let putCalled = false
    let release: () => void = () => undefined
    mockFetch({
      'GET /api/push/key': () => json(200, { public_key: 'AQID' }),
      'PUT /api/push/subscription': () => {
        putCalled = true
        return new Promise((resolve) => {
          release = () => {
            registered = true
            order.push('put')
            resolve(json(204, null))
          }
        })
      },
      // 서버 상태 그대로: 등록 전에는 구독 없음.
      'GET /api/push/subscription': () => {
        order.push('get')
        return json(200, { endpoint: registered ? sub.endpoint : null })
      },
    })
    const subscribed = subscribeDevice()
    await vi.waitFor(() => {
      expect(putCalled).toBe(true)
    })
    const reconciled = reconcileDevice()
    await new Promise((r) => setTimeout(r, 20))
    expect(order).toEqual([])
    release()
    await subscribed
    expect(await reconciled).toBe(true)
    expect(order).toEqual(['put', 'get'])
    expect(sub.unsubscribe).not.toHaveBeenCalled()
  })

  it('throws_and_unsubscribes_when_keys_missing', async () => {
    const sub = {
      endpoint: 'https://fcm.googleapis.com/x',
      unsubscribe: vi.fn().mockResolvedValue(true),
      toJSON: () => ({ endpoint: 'https://fcm.googleapis.com/x' }),
    }
    installPush({ current: null }).subscribe.mockResolvedValue(sub)
    mockFetch({ 'GET /api/push/key': () => json(200, { public_key: 'AQID' }) })
    await expect(subscribeDevice()).rejects.toThrow('push.invalidSubscription')
    expect(sub.unsubscribe).toHaveBeenCalled()
  })

  it('unsubscribes_browser_when_put_fails', async () => {
    const sub = fakeSub('https://fcm.googleapis.com/x')
    installPush({ current: null, subscribeResult: sub })
    mockFetch({
      'GET /api/push/key': () => json(200, { public_key: 'AQID' }),
      'PUT /api/push/subscription': () =>
        json(400, { error: 'push.endpointNotAllowed', params: {} }),
    })
    await expect(subscribeDevice()).rejects.toThrow('push.endpointNotAllowed')
    expect(sub.unsubscribe).toHaveBeenCalled()
  })
})

describe('reconcileDevice', () => {
  it('shares_running_check', async () => {
    installPush({ current: fakeSub('https://fcm.googleapis.com/a') })
    let gets = 0
    mockFetch({
      'GET /api/push/subscription': () => {
        gets += 1
        return json(200, { endpoint: 'https://fcm.googleapis.com/a' })
      },
    })
    expect(await Promise.all([reconcileDevice(), reconcileDevice()])).toEqual([
      true,
      true,
    ])
    expect(gets).toBe(1)
  })
})

describe('unsubscribeBrowserOnly', () => {
  it('gives_up_after_three_seconds', async () => {
    const { getRegistration } = installPush({ current: null })
    getRegistration.mockReturnValue(new Promise(() => undefined))
    vi.useFakeTimers()
    try {
      let done = false
      const p = unsubscribeBrowserOnly().then(() => {
        done = true
      })
      await vi.advanceTimersByTimeAsync(2999)
      expect(done).toBe(false)
      await vi.advanceTimersByTimeAsync(1)
      await p
      expect(done).toBe(true)
    } finally {
      vi.useRealTimers()
    }
  })
})
