import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button, buttonVariants } from '@/components/ui/button'

const KEY_FILE_NAME = 'chagokmoa-recovery-key.txt'

// 복구 키 크게 표시, 복사, 텍스트 파일 받기.
export function KeyBox({ recoveryKey }: { recoveryKey: string }) {
  const { t } = useTranslation()
  const [copyFailed, setCopyFailed] = useState(false)
  // 짧은 텍스트라 data URL. 해제할 객체 URL 없음.
  const file = `data:text/plain;charset=utf-8,${encodeURIComponent(
    `${t('backup.keyFileHeading')}\n\n${recoveryKey}\n`,
  )}`

  async function copy() {
    try {
      await navigator.clipboard.writeText(recoveryKey)
      setCopyFailed(false)
      toast(t('common.copied'))
    } catch {
      setCopyFailed(true)
    }
  }

  return (
    <div className="grid gap-3 rounded-xl bg-muted p-4">
      <p className="text-sm font-semibold">{t('backup.keyLabel')}</p>
      <p className="font-mono text-lg font-bold tracking-wide break-all select-all sm:text-xl">
        {recoveryKey}
      </p>
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" onClick={() => void copy()}>
          {t('common.copy')}
        </Button>
        <a
          href={file}
          download={KEY_FILE_NAME}
          className={buttonVariants({ variant: 'outline' })}
        >
          {t('backup.keyDownload')}
        </a>
      </div>
      <p className="text-sm text-muted-foreground">{t('backup.keyWarn')}</p>
      {copyFailed && (
        <p role="alert" className="text-sm text-destructive">
          {t('backup.copyFailed')}
        </p>
      )}
    </div>
  )
}
