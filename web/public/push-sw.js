// workbox generateSW 가 importScripts 로 불러오는 일반 스크립트(모듈 아님).
// 앱 쪽 메시지 규약: cgm.push(수신), cgm.navigate(클릭).

const DEFAULT_TITLE = '차곡모아'
const DEFAULT_URL = '/notifications'

// 깨진 본문이나 빠진 필드도 알림은 표시. 표시 없는 푸시 반복 시 Safari 구독 해지.
function readPayload(event) {
  let data = null
  try {
    data = event.data ? event.data.json() : null
  } catch {
    // 깨진 JSON
  }
  const d = data && typeof data === 'object' ? data : {}
  const text = (v) => (typeof v === 'string' ? v : '')
  return {
    title: text(d.title) || DEFAULT_TITLE,
    body: text(d.body),
    url: text(d.url) || DEFAULT_URL,
  }
}

self.addEventListener('push', (event) => {
  const { title, body, url } = readPayload(event)
  event.waitUntil(
    (async () => {
      await self.registration.showNotification(title, {
        body,
        icon: '/icon-192.png',
        data: { url },
      })
      const windows = await self.clients.matchAll({
        type: 'window',
        includeUncontrolled: true,
      })
      for (const w of windows) w.postMessage({ type: 'cgm.push' })
    })(),
  )
})

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const url = (event.notification.data && event.notification.data.url) || '/'
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({
        type: 'window',
        includeUncontrolled: true,
      })
      const target = windows.find((w) => w.focused) || windows[0]
      if (target) {
        // 이동 먼저. 포커스는 브라우저가 거부 가능.
        target.postMessage({ type: 'cgm.navigate', url })
        try {
          await target.focus()
        } catch {
          // 포커스 거부 무시
        }
      } else {
        await self.clients.openWindow(url)
      }
    })(),
  )
})

// 푸시 서비스가 구독을 갱신한 경우. 서버 구독 교체.
// 실패는 무시. 다음 앱 시작 확인에서 브라우저 구독 해지, 꺼진 상태로 표시.
self.addEventListener('pushsubscriptionchange', (event) => {
  event.waitUntil(
    (async () => {
      let sub = event.newSubscription
      if (!sub && event.oldSubscription) {
        sub = await self.registration.pushManager.subscribe(
          event.oldSubscription.options,
        )
      }
      if (!sub) return
      const json = sub.toJSON()
      const keys = json.keys || {}
      if (!json.endpoint || !keys.p256dh || !keys.auth) return
      await fetch('/api/push/subscription', {
        method: 'PUT',
        credentials: 'same-origin',
        headers: {
          'Content-Type': 'application/json',
          'X-Requested-With': 'Chagokmoa',
        },
        body: JSON.stringify({
          endpoint: json.endpoint,
          p256dh: keys.p256dh,
          auth: keys.auth,
        }),
      }).catch(() => undefined)
    })(),
  )
})
