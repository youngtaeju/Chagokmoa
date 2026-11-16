import { useState, type ComponentProps } from 'react'
import { useTranslation } from 'react-i18next'
import IconVisibility from '~icons/material-symbols/visibility-outline-rounded'
import IconVisibilityOff from '~icons/material-symbols/visibility-off-outline-rounded'
import { cn } from '@/lib/utils'
import { Field } from './Field'

// 보기 버튼이 있는 비밀번호 칸. 값은 유지하고 type 만 전환.
export function PasswordField(
  props: Omit<ComponentProps<typeof Field>, 'type'>,
) {
  const { t } = useTranslation()
  const [shown, setShown] = useState(false)
  return (
    <Field
      {...props}
      type={shown ? 'text' : 'password'}
      end={
        <button
          type="button"
          aria-pressed={shown}
          aria-label={shown ? t('auth.hidePassword') : t('auth.showPassword')}
          onClick={() => {
            setShown((v) => !v)
          }}
          className={cn(
            'absolute grid size-11 place-items-center rounded-lg text-muted-foreground focus-visible:outline-2 focus-visible:outline-ring',
            props.tall ? 'right-1 top-1' : 'right-0 top-0',
          )}
        >
          {shown ? (
            <IconVisibilityOff className="size-[22px]" />
          ) : (
            <IconVisibility className="size-[22px]" />
          )}
        </button>
      }
    />
  )
}
