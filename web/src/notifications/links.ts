import type { AppNotification } from '@/api/types'
import { addDays } from '@/lib/dates'
import { todayInZone } from '@/viewer/zoned'

export type NotificationLink =
  | {
      to: '/photos/$id'
      params: { id: string }
      search?: { panel: 'comments' }
    }
  | { to: '/days/$date'; params: { date: string } }
  | { to: '/stories/$id'; params: { id: string } }
  | { to: '/milestones/$id'; params: { id: string } }
  | { to: '/settings/backup' }

// 스펙 §6 이동 표. 대상 요약 없음(서버가 휴지통 등으로 비움)이면 null.
export function notificationLink(n: AppNotification): NotificationLink | null {
  switch (n.kind) {
    case 'upload':
      if (!n.asset) return null
      return n.count > 1
        ? { to: '/days/$date', params: { date: n.asset.local_date } }
        : { to: '/photos/$id', params: { id: n.asset.id } }
    case 'comment':
    case 'mention':
      // 사진 댓글과 그 멘션은 asset, 이야기 댓글과 본문 멘션은 story.
      if (n.asset) {
        return {
          to: '/photos/$id',
          params: { id: n.asset.id },
          search: { panel: 'comments' },
        }
      }
      return n.story ? { to: '/stories/$id', params: { id: n.story.id } } : null
    case 'story':
      return n.story ? { to: '/stories/$id', params: { id: n.story.id } } : null
    case 'milestone':
      return n.milestone
        ? { to: '/milestones/$id', params: { id: n.milestone.id } }
        : null
    case 'backup':
      return { to: '/settings/backup' }
  }
}

export type Section = 'today' | 'week' | 'earlier'

// 인스턴스 시간대 날짜 기준. 이번 주 = 오늘 제외 최근 7일. 시계 차이로 미래면 오늘.
export function sectionOf(
  updatedAt: string,
  today: string,
  timezone: string,
): Section {
  const day = todayInZone(timezone, new Date(updatedAt))
  if (day >= today) return 'today'
  return day >= addDays(today, -7) ? 'week' : 'earlier'
}
