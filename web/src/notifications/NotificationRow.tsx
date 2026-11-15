import { Link } from '@tanstack/react-router'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'
import type { AssetSummary, AppNotification, User } from '@/api/types'
import { relativeTime } from '@/comments/relativeTime'
import { toSegments } from '@/comments/mentions'
import { cn } from '@/lib/utils'
import { milestoneLabel } from '@/milestones/labels'
import { Blurhash } from '@/timeline/Blurhash'
import { thumbUrl } from '@/timeline/media'
import { formatInZone } from '@/viewer/zoned'
import { notificationLink } from './links'

// 서버 100자 자르기로 끝에 남은 미완성 멘션 토큰.
const CUT_TOKEN = /<@[0-9a-f-]{0,36}$/

// 멘션 토큰 → `@현재 이름` 평문. 잘린 끝 토큰은 버림.
function plain(t: TFunction, body: string, users: User[]): string {
  return toSegments(
    body.replace(CUT_TOKEN, ''),
    users,
    t('comments.unknownUser'),
  )
    .map((s) => (s.kind === 'text' ? s.text : `@${s.name}`))
    .join('')
}

function sentence(
  t: TFunction,
  n: AppNotification,
  meId: string | undefined,
  users: User[],
): string {
  const name = n.actor.display_name
  switch (n.kind) {
    case 'upload':
      // 영상 포함 1장도 "사진".
      return n.count > 1
        ? t('notifications.uploadMany', { name, count: n.count })
        : t('notifications.uploadOne', { name })
    case 'comment': {
      const text = plain(t, n.comment?.excerpt ?? '', users)
      if (!n.asset) return t('notifications.commentStory', { name, text })
      return n.asset.uploaded_by === meId
        ? t('notifications.commentMyPhoto', { name, text })
        : t('notifications.commentPhoto', { name, text })
    }
    case 'mention': {
      // 이야기 본문 멘션은 댓글 없이 이야기 발췌.
      const body = n.comment?.excerpt ?? n.story?.excerpt ?? ''
      return t('notifications.mention', { name, text: plain(t, body, users) })
    }
    case 'story':
      return t('notifications.story', {
        name,
        text: plain(t, n.story?.excerpt ?? '', users),
      })
    case 'milestone':
      return t('notifications.milestone', {
        name,
        label: n.milestone ? milestoneLabel(t, n.milestone) : '',
      })
  }
}

function Thumb({ a }: { a: AssetSummary }) {
  return (
    <span
      aria-hidden
      className="relative size-12 shrink-0 overflow-hidden rounded-md bg-muted"
      style={a.dominant_color ? { backgroundColor: a.dominant_color } : {}}
    >
      {a.blurhash && (
        <Blurhash hash={a.blurhash} className="absolute inset-0 size-full" />
      )}
      {a.status === 'ready' && (
        <img
          src={thumbUrl(a.id)}
          alt=""
          loading="lazy"
          decoding="async"
          className="absolute inset-0 size-full object-cover"
        />
      )}
    </span>
  )
}

// 행동자 첫 글자, 문장, 상대 시각, 대상 썸네일. 새 줄은 옅은 살구 바탕과 왼쪽 점.
export function NotificationRow({
  n,
  isNew,
  meId,
  users,
  now,
  timezone,
}: {
  n: AppNotification
  isNew: boolean
  meId: string | undefined
  users: User[]
  now: Date
  timezone: string
}) {
  const { t, i18n } = useTranslation()
  const link = notificationLink(n)
  const thumb = n.asset ?? n.story?.cover ?? n.milestone?.cover ?? null
  const className = cn(
    'relative flex min-h-16 items-center gap-3 rounded-lg py-2 pr-2 pl-5',
    isNew && 'bg-accent-soft/60',
    link && 'hover:bg-accent',
  )
  const body = (
    <>
      {isNew && (
        <>
          <span
            aria-hidden
            className="absolute top-1/2 left-1.5 size-2 -translate-y-1/2 rounded-full bg-primary"
          />
          <span className="sr-only">{t('notifications.new')}</span>
        </>
      )}
      <span
        aria-hidden
        className="inline-flex size-10 shrink-0 items-center justify-center rounded-full bg-accent-soft text-sm font-bold text-accent-ink"
      >
        {Array.from(n.actor.display_name)[0] ?? '?'}
      </span>
      <span className="grid min-w-0 flex-1 gap-0.5">
        <span className="line-clamp-2 text-sm break-words">
          {sentence(t, n, meId, users)}
        </span>
        <time
          dateTime={n.updated_at}
          title={formatInZone(n.updated_at, timezone, i18n.language)}
          className="text-xs text-muted-foreground tabular-nums"
        >
          {relativeTime(n.updated_at, now, i18n.language, t)}
        </time>
      </span>
      {thumb && <Thumb a={thumb} />}
    </>
  )
  return (
    <li>
      {link ? (
        <Link {...link} data-new={isNew} className={className}>
          {body}
        </Link>
      ) : (
        <div data-new={isNew} className={className}>
          {body}
        </div>
      )}
    </li>
  )
}
