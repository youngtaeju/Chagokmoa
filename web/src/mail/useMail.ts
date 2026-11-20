import { queryOptions } from '@tanstack/react-query'
import { api, call } from '@/api/client'

export const mailQueryKey = ['mail'] as const

export const mailQuery = queryOptions({
  queryKey: mailQueryKey,
  queryFn: () => call(api.GET('/api/mail')),
})
