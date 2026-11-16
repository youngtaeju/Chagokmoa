import { api, call } from '@/api/client'

export type PushSupport = 'supported' | 'ios-install' | 'denied' | 'unsupported'

function isStandalone(): boolean {
  const nav = navigator as Navigator & { standalone?: boolean }
  return (
    nav.standalone === true || matchMedia('(display-mode: standalone)').matches
  )
}

function isIos(): boolean {
  const ua = navigator.userAgent
  return (
    /iPhone|iPad|iPod/.test(ua) ||
    // iPadOS 는 데스크톱 UA.
    (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)
  )
}

export function pushSupport(): PushSupport {
  const capable =
    window.isSecureContext &&
    'serviceWorker' in navigator &&
    'PushManager' in window &&
    'Notification' in window
  if (!capable) {
    // iOS 는 홈 화면에 설치한 앱에서만 푸시 가능.
    return isIos() && !isStandalone() ? 'ios-install' : 'unsupported'
  }
  return Notification.permission === 'denied' ? 'denied' : 'supported'
}

// 서비스 워커가 없으면 구독도 없음. 개발 서버처럼 미등록인 경우 대기 방지.
export async function currentSubscription(): Promise<PushSubscription | null> {
  const reg = await navigator.serviceWorker.getRegistration()
  return (await reg?.pushManager.getSubscription()) ?? null
}

function keyBytes(b64url: string): Uint8Array<ArrayBuffer> {
  const b64 = b64url.replace(/-/g, '+').replace(/_/g, '/')
  const raw = atob(b64.padEnd(Math.ceil(b64.length / 4) * 4, '='))
  return Uint8Array.from(raw, (c) => c.charCodeAt(0))
}

// 확인, 구독, 해지를 한 줄로 직렬화. 확인이 등록 전 새 구독을 이전 구독으로 보고 해지하는 경합 방지.
let queue: Promise<unknown> = Promise.resolve()

function serial<T>(fn: () => Promise<T>): Promise<T> {
  // 앞 작업 실패와 무관하게 진행.
  const run = queue.then(fn, fn)
  queue = run.catch(() => undefined)
  return run
}

export async function subscribeDevice(): Promise<void> {
  // 권한 요청이 먼저. iOS 는 사용자 누름 직후에만 권한 창 허용.
  if ((await Notification.requestPermission()) !== 'granted') {
    throw new Error('push.permissionDenied')
  }
  await serial(subscribeGranted)
}

async function subscribeGranted(): Promise<void> {
  const { public_key } = await call(api.GET('/api/push/key'))
  const reg = await navigator.serviceWorker.ready
  const sub = await reg.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: keyBytes(public_key),
  })
  const { endpoint, keys } = sub.toJSON()
  try {
    if (!keys?.p256dh || !keys.auth) {
      throw new Error('push.invalidSubscription')
    }
    await call(
      api.PUT('/api/push/subscription', {
        body: {
          endpoint: endpoint ?? sub.endpoint,
          p256dh: keys.p256dh,
          auth: keys.auth,
        },
      }),
    )
  } catch (e) {
    // 서버에 없는 구독이 브라우저에 남지 않게.
    await sub.unsubscribe().catch(() => undefined)
    throw e
  }
}

export function unsubscribeDevice(): Promise<void> {
  return serial(async () => {
    await (await currentSubscription())?.unsubscribe()
    await call(api.DELETE('/api/push/subscription'))
  })
}

const LOGOUT_UNSUBSCRIBE_MS = 3000

// 로그아웃용. 실패하거나 서비스 워커 응답이 늦어도 로그아웃을 막지 않음.
export async function unsubscribeBrowserOnly(): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<void>((resolve) => {
    timer = setTimeout(resolve, LOGOUT_UNSUBSCRIBE_MS)
  })
  try {
    if (pushSupport() === 'unsupported') return
    await Promise.race([
      currentSubscription().then((sub) => sub?.unsubscribe()),
      timeout,
    ])
  } catch {
    // 무시
  } finally {
    clearTimeout(timer)
  }
}

async function checkDevice(): Promise<boolean> {
  const sub = await currentSubscription()
  if (!sub) return false
  const { endpoint } = await call(api.GET('/api/push/subscription'))
  if (endpoint === sub.endpoint) return true
  await sub.unsubscribe()
  return false
}

// 브라우저 구독이 이 세션의 서버 구독과 같은지 확인. 다르면(다른 계정 잔존 등) 브라우저 구독 해지.
// 반환: 이 기기가 이 계정으로 켜져 있는지.
// 대기 중이거나 진행 중인 확인 하나를 앱 시작 확인과 화면이 공유.
let reconciling: Promise<boolean> | null = null

export function reconcileDevice(): Promise<boolean> {
  reconciling ??= serial(checkDevice).finally(() => {
    reconciling = null
  })
  return reconciling
}
