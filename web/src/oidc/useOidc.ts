import { queryOptions } from '@tanstack/react-query'
import { api, call } from '@/api/client'

export const oidcQueryKey = ['oidc'] as const

export const oidcQuery = queryOptions({
  queryKey: oidcQueryKey,
  queryFn: () => call(api.GET('/api/oidc')),
})
