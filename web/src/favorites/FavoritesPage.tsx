import { useTranslation } from 'react-i18next'
import { SubPage } from '@/components/SubPage'
import { useSettings } from '@/settings/useSettings'
import { AssetGrid } from '@/timeline/AssetGrid'
import { useFavorites } from './useFavorites'

export function FavoritesPage() {
  const { t } = useTranslation()
  const timezone = useSettings()?.timezone
  const favorites = useFavorites()
  return (
    <SubPage title={t('albums.favorites')} backTo="/albums">
      <AssetGrid
        {...favorites}
        timezone={timezone}
        linkSearch={{ from: 'favorites' }}
        empty={t('favorites.empty')}
      />
    </SubPage>
  )
}
