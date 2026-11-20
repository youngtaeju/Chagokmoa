import { useId, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

// 만든 링크 표시. 읽기 전용 주소 칸, 공유(지원 시), 복사.
export function LinkShareBox({
  url,
  label,
  shareTitle,
  shareText,
  hint,
}: {
  url: string
  label: string
  shareTitle: string
  shareText: string
  hint: string
}) {
  const { t } = useTranslation()
  const id = useId()
  const urlInput = useRef<HTMLInputElement>(null)
  const [copyFailed, setCopyFailed] = useState(false)

  async function copy(silent = false) {
    try {
      await navigator.clipboard.writeText(url)
      setCopyFailed(false)
      toast(silent ? t('members.linkCopied') : t('common.copied'))
    } catch {
      setCopyFailed(true)
      urlInput.current?.select()
    }
  }

  // 버튼 클릭 안에서만 호출(iOS 는 사용자 동작 직후여야 허용). 사용자 취소(AbortError)는 무시, 그 외 실패는 복사.
  async function share() {
    try {
      await navigator.share({ title: shareTitle, text: shareText, url })
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') return
      await copy(true)
    }
  }

  return (
    <div className="grid gap-2 rounded-xl bg-muted p-3">
      <label htmlFor={id} className="text-sm font-semibold">
        {label}
      </label>
      <div className="flex gap-2">
        <Input
          id={id}
          ref={urlInput}
          readOnly
          value={url}
          onFocus={(e) => {
            e.currentTarget.select()
          }}
        />
        {typeof navigator.share === 'function' && (
          <Button variant="outline" onClick={() => void share()}>
            {t('members.share')}
          </Button>
        )}
        <Button variant="outline" onClick={() => void copy()}>
          {t('common.copy')}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{hint}</p>
      {copyFailed && (
        <p role="alert" className="text-sm text-destructive">
          {t('members.copyFailed')}
        </p>
      )}
    </div>
  )
}

// 링크와 함께 메일 발송 결과 한 줄. 실패해도 링크는 그대로 공유 가능.
export function MailResult({ email, sent }: { email: string; sent: boolean }) {
  const { t } = useTranslation()
  return (
    <p role="status" className={sent ? 'text-sm' : 'text-sm text-destructive'}>
      {sent ? t('members.mailSent', { email }) : t('members.mailFailed')}
    </p>
  )
}
