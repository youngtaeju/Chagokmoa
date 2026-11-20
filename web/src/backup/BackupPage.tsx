import { useTranslation } from 'react-i18next'
import { FormError } from '@/components/Field'
import { SubPage } from '@/components/SubPage'
import { EnableFlow } from './EnableFlow'
import { KeySection } from './KeySection'
import { RemoteSection } from './RemoteSection'
import { SettingsSection } from './SettingsSection'
import { StatusSection } from './StatusSection'
import { useBackupStatus } from './status'

export function BackupPage() {
  const { t } = useTranslation()
  const { data: status, error } = useBackupStatus()
  return (
    <SubPage section={t('me.system')} title={t('me.backup')}>
      <div className="mx-auto grid w-full max-w-xl gap-6">
        <FormError error={error} />
        {status &&
          (status.key_set ? (
            <>
              <StatusSection status={status} />
              <SettingsSection status={status} />
              <RemoteSection status={status} />
              <KeySection />
            </>
          ) : (
            <EnableFlow status={status} />
          ))}
      </div>
    </SubPage>
  )
}
