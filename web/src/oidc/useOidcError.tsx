import { useNavigate, useSearch } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

// 서버 callback 이 붙이는 키 계열. 그 밖의 값은 일반 실패 문구.
const KEY_PREFIX = /^(oidc|auth|invite)\./

// 주소의 oidc_error 를 번역 문구로 잡아 두고 주소에서는 제거(새로고침 시 재표시 방지).
export function useOidcError(): string | null {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { oidc_error: key } = useSearch({ strict: false })
  // 첫 렌더 값 보존. 주소에서 지워도 문구 유지.
  const [shown] = useState(key)

  useEffect(() => {
    if (!key) return
    void navigate({
      to: '.',
      search: (prev: Record<string, unknown>) => ({
        ...prev,
        oidc_error: undefined,
      }),
      replace: true,
    })
  }, [key, navigate])

  if (!shown) return null
  const msg = KEY_PREFIX.test(shown)
    ? t(`errors.${shown}`, { defaultValue: '' })
    : ''
  return msg || t('errors.oidc.failed')
}

export function OidcErrorNotice({ message }: { message: string | null }) {
  if (!message) return null
  return (
    <p role="alert" className="text-sm text-destructive">
      {message}
    </p>
  )
}
