import type { components } from '@/api/schema'

export type MailSecurity = components['schemas']['MailSecurity']
export type MailPresetId = 'gmail' | 'naver'

export const MAIL_PRESETS: {
  id: MailPresetId
  host: string
  port: number
  security: 'starttls'
}[] = [
  { id: 'gmail', host: 'smtp.gmail.com', port: 587, security: 'starttls' },
  { id: 'naver', host: 'smtp.naver.com', port: 587, security: 'starttls' },
]

// 화면 편의용 분류. 저장 값에는 없고 값이 프리셋과 같으면 그 이름.
export function presetOf(
  host: string,
  port: number,
  security: MailSecurity,
): MailPresetId | 'custom' {
  const hit = MAIL_PRESETS.find(
    (p) =>
      p.host === host.trim().toLowerCase() &&
      p.port === port &&
      p.security === security,
  )
  return hit?.id ?? 'custom'
}
