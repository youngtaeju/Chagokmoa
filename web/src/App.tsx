import { QueryClientProvider, type QueryClient } from '@tanstack/react-query'
import { RouterProvider } from '@tanstack/react-router'
import { useState } from 'react'
import { Toaster } from '@/components/ui/sonner'
import { createQueryClient } from '@/auth/session'
import { useReconcilePushDevice } from '@/push/usePushDevice'
import { useSwMessages } from '@/push/useSwMessages'
import { UpdatePrompt } from '@/pwa/UpdatePrompt'
import { createApp, type AppRouter } from '@/router'
import { useTheme } from '@/theme/theme'

// QueryClient 안에서 서비스 워커 메시지 수신과 푸시 구독 점검.
function PushBridge({ router }: { router: AppRouter }) {
  useSwMessages(router)
  useReconcilePushDevice()
  return null
}

export function AppView({
  queryClient,
  router,
}: {
  queryClient: QueryClient
  router: AppRouter
}) {
  const [theme] = useTheme()
  return (
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
      <Toaster theme={theme} />
      <UpdatePrompt />
      <PushBridge router={router} />
    </QueryClientProvider>
  )
}

export default function App() {
  const [app] = useState(() => createApp(createQueryClient))
  return <AppView {...app} />
}
