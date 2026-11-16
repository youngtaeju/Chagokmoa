import { useId, type ComponentProps, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ApiError } from '@/api/errors'
import { errorMessage } from '@/i18n'
import { cn } from '@/lib/utils'

export function Field({
  label,
  hint,
  error,
  icon,
  tall,
  end,
  className,
  ...props
}: {
  label: string
  hint?: string
  // 입력칸 아래 인라인 오류 문구.
  error?: string | null
  // 입력칸 왼쪽 장식 아이콘(왼쪽 여백 46px).
  icon?: ReactNode
  // 높이 52px, 글자 16px.
  tall?: boolean
  // 입력칸 오른쪽 겹침 요소(비밀번호 보기 버튼). PasswordField 전용.
  end?: ReactNode
} & ComponentProps<typeof Input>) {
  const id = useId()
  const hintId = useId()
  const errorId = useId()
  const describedBy =
    [hint ? hintId : null, error ? errorId : null].filter(Boolean).join(' ') ||
    undefined
  const input = (
    <Input
      id={id}
      aria-describedby={describedBy}
      aria-invalid={error ? true : undefined}
      className={cn(
        tall && 'h-[52px] text-base md:text-base',
        icon && 'pl-[46px]',
        end && 'pr-[52px]',
        className,
      )}
      {...props}
    />
  )
  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{label}</Label>
      {icon || end ? (
        <div className="relative">
          {icon && (
            <span
              aria-hidden="true"
              className={cn(
                'pointer-events-none absolute left-3.5 grid place-items-center text-muted-foreground [&_svg]:size-5',
                tall ? 'top-0 h-[52px]' : 'top-0 h-11',
              )}
            >
              {icon}
            </span>
          )}
          {input}
          {end}
        </div>
      ) : (
        input
      )}
      {hint && (
        <p id={hintId} className="text-xs text-muted-foreground">
          {hint}
        </p>
      )}
      {error && (
        <p id={errorId} role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </div>
  )
}

// auth.disabled 는 로그인 응답으로도 오므로 숨기지 않음.
function isSessionRequired(e: unknown): boolean {
  return e instanceof ApiError && e.status === 401 && e.key === 'auth.required'
}

// 서버 에러 키를 번역해 표시. 클라이언트 검증 문구는 없음. 세션 만료는 전역 로그아웃이 처리하므로 숨김.
export function FormError({ error }: { error: unknown }) {
  const { t } = useTranslation()
  if (error == null || isSessionRequired(error)) return null
  return (
    <p role="alert" className="text-sm text-destructive">
      {errorMessage(t, error)}
    </p>
  )
}

export function formText(form: FormData, name: string): string {
  const v = form.get(name)
  return typeof v === 'string' ? v : ''
}
