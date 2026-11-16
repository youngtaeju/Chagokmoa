import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from '@/components/ui/sheet'

// 이메일 발송이 없으므로 관리자 요청 안내만 표시.
export function ForgotPasswordSheet() {
  const { t } = useTranslation()
  return (
    <Sheet>
      <SheetTrigger asChild>
        <Button
          type="button"
          variant="link"
          className="min-h-11 px-3 text-[15px] font-semibold underline"
        >
          {t('auth.login.forgot')}
        </Button>
      </SheetTrigger>
      <SheetContent side="bottom" className="mx-auto max-w-[400px]">
        <SheetHeader className="pb-6">
          <SheetTitle>{t('auth.login.forgotTitle')}</SheetTitle>
          <SheetDescription>{t('auth.login.forgotAskAdmin')}</SheetDescription>
        </SheetHeader>
      </SheetContent>
    </Sheet>
  )
}
