import * as React from 'react'
import { cn } from '@/lib/utils'

// 네이티브 button 기반. Space/Enter 는 기본 동작.
function Switch({
  checked,
  onCheckedChange,
  className,
  ...props
}: Omit<React.ComponentProps<'button'>, 'onClick' | 'role'> & {
  checked: boolean
  onCheckedChange: (checked: boolean) => void
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      data-slot="switch"
      onClick={() => {
        onCheckedChange(!checked)
      }}
      className={cn(
        'relative inline-flex h-7 w-12 shrink-0 items-center rounded-full bg-input transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50 aria-checked:bg-primary',
        className,
      )}
      {...props}
    >
      <span
        aria-hidden
        className={cn(
          'pointer-events-none size-5 rounded-full bg-background shadow transition-transform',
          checked ? 'translate-x-6' : 'translate-x-1',
        )}
      />
    </button>
  )
}

export { Switch }
