import { useTranslation } from 'react-i18next'
import { MeSubPage } from '@/me/MeSubPage'
import { RegenSection } from './RegenSection'

export function BackupPage() {
  const { t } = useTranslation()
  return (
    <MeSubPage title={t('me.backup')}>
      <div className="mx-auto grid w-full max-w-xl gap-6">
        <RegenSection />
      </div>
    </MeSubPage>
  )
}
