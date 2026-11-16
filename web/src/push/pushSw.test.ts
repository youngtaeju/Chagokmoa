import { readFileSync } from 'node:fs'
import path from 'node:path'
import { runInNewContext } from 'node:vm'
import { describe, expect, it, vi } from 'vitest'

type Handler = (event: unknown) => void

const source = readFileSync(
  path.resolve(import.meta.dirname, '../../public/push-sw.js'),
  'utf8',
)

interface FakeWindow {
  focused?: boolean
  focus: () => Promise<void>
  postMessage: (m: unknown) => void
}

function load(windows: FakeWindow[]) {
  const handlers = new Map<string, Handler>()
  const showNotification = vi.fn().mockResolvedValue(undefined)
  const openWindow = vi.fn().mockResolvedValue(undefined)
  const subscribe = vi.fn()
  const self = {
    addEventListener: (type: string, h: Handler) => handlers.set(type, h),
    registration: { showNotification, pushManager: { subscribe } },
    clients: { matchAll: vi.fn().mockResolvedValue(windows), openWindow },
  }
  runInNewContext(source, {
    self,
    fetch: (...a: Parameters<typeof fetch>) => globalThis.fetch(...a),
  })
  // 이벤트: waitUntil 로 받은 약속을 돌려받음.
  const fire = async (type: string, event: Record<string, unknown>) => {
    let p: Promise<unknown> = Promise.resolve()
    handlers.get(type)?.({
      ...event,
      waitUntil: (x: Promise<unknown>) => (p = x),
    })
    await p
  }
  return { fire, showNotification, openWindow, subscribe, self }
}

describe('push-sw', () => {
  it('push_shows_notification_and_notifies_clients', async () => {
    const w = { focus: vi.fn(), postMessage: vi.fn() }
    const { fire, showNotification } = load([w])
    await fire('push', {
      data: { json: () => ({ title: 'T', body: 'B', url: '/x' }) },
    })
    expect(showNotification).toHaveBeenCalledWith('T', {
      body: 'B',
      icon: '/icon-192.png',
      data: { url: '/x' },
    })
    expect(w.postMessage).toHaveBeenCalledWith({ type: 'cgm.push' })
  })

  it('push_with_invalid_json_shows_default_notification', async () => {
    const { fire, showNotification } = load([])
    await fire('push', {
      data: {
        json: () => {
          throw new SyntaxError('bad json')
        },
      },
    })
    expect(showNotification).toHaveBeenCalledWith('차곡모아', {
      body: '',
      icon: '/icon-192.png',
      data: { url: '/notifications' },
    })
  })

  it('push_without_title_shows_default_title', async () => {
    const { fire, showNotification } = load([])
    await fire('push', { data: { json: () => ({ body: 'B' }) } })
    expect(showNotification).toHaveBeenCalledWith('차곡모아', {
      body: 'B',
      icon: '/icon-192.png',
      data: { url: '/notifications' },
    })
  })

  it('click_focuses_existing_window_and_navigates', async () => {
    const w = {
      focus: vi.fn().mockResolvedValue(undefined),
      postMessage: vi.fn(),
    }
    const { fire, openWindow } = load([w])
    const close = vi.fn()
    await fire('notificationclick', {
      notification: { close, data: { url: '/notifications' } },
    })
    expect(close).toHaveBeenCalled()
    expect(w.focus).toHaveBeenCalled()
    expect(w.postMessage).toHaveBeenCalledWith({
      type: 'cgm.navigate',
      url: '/notifications',
    })
    expect(openWindow).not.toHaveBeenCalled()
  })

  it('click_prefers_focused_window', async () => {
    const a = {
      focus: vi.fn().mockResolvedValue(undefined),
      postMessage: vi.fn(),
    }
    const b = {
      focused: true,
      focus: vi.fn().mockResolvedValue(undefined),
      postMessage: vi.fn(),
    }
    const { fire } = load([a, b])
    await fire('notificationclick', {
      notification: { close: vi.fn(), data: { url: '/x' } },
    })
    expect(b.postMessage).toHaveBeenCalledWith({
      type: 'cgm.navigate',
      url: '/x',
    })
    expect(a.postMessage).not.toHaveBeenCalled()
  })

  it('click_navigates_even_when_focus_rejected', async () => {
    const w = {
      focus: vi.fn().mockRejectedValue(new Error('InvalidAccessError')),
      postMessage: vi.fn(),
    }
    const { fire } = load([w])
    await fire('notificationclick', {
      notification: { close: vi.fn(), data: { url: '/x' } },
    })
    expect(w.postMessage).toHaveBeenCalledWith({
      type: 'cgm.navigate',
      url: '/x',
    })
  })

  it('click_opens_window_when_none', async () => {
    const { fire, openWindow } = load([])
    await fire('notificationclick', {
      notification: { close: vi.fn(), data: { url: '/a' } },
    })
    expect(openWindow).toHaveBeenCalledWith('/a')
  })

  it('subscription_change_puts_new_subscription', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 204 }))
    vi.stubGlobal('fetch', fetchMock)
    const { fire } = load([])
    await fire('pushsubscriptionchange', {
      newSubscription: {
        toJSON: () => ({
          endpoint: 'https://e',
          keys: { p256dh: 'P', auth: 'A' },
        }),
      },
    })
    expect(fetchMock).toHaveBeenCalledWith('/api/push/subscription', {
      method: 'PUT',
      credentials: 'same-origin',
      headers: {
        'Content-Type': 'application/json',
        'X-Requested-With': 'Chagokmoa',
      },
      body: JSON.stringify({ endpoint: 'https://e', p256dh: 'P', auth: 'A' }),
    })
  })

  it('subscription_change_resubscribes_from_old_options', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response(null, { status: 204 })),
    )
    const { fire, subscribe } = load([])
    subscribe.mockResolvedValue({
      toJSON: () => ({
        endpoint: 'https://n',
        keys: { p256dh: 'P', auth: 'A' },
      }),
    })
    const options = { userVisibleOnly: true }
    await fire('pushsubscriptionchange', { oldSubscription: { options } })
    expect(subscribe).toHaveBeenCalledWith(options)
  })

  it('subscription_change_without_keys_skips_request', async () => {
    const fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
    const { fire } = load([])
    await fire('pushsubscriptionchange', {
      newSubscription: { toJSON: () => ({ endpoint: 'https://e' }) },
    })
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('subscription_change_ignores_failed_response', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response(null, { status: 400 })),
    )
    const { fire } = load([])
    await expect(
      fire('pushsubscriptionchange', {
        newSubscription: {
          toJSON: () => ({
            endpoint: 'https://e',
            keys: { p256dh: 'P', auth: 'A' },
          }),
        },
      }),
    ).resolves.toBeUndefined()
  })
})
