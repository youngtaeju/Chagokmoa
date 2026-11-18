import { describe, expect, it } from 'vitest'
import { assetSummaryFixture, notificationFixture } from '@/test/fixtures'
import { notificationLink, sectionOf } from './links'

const story = { id: 's', excerpt: '오늘 처음 웃었어요', cover: null }

describe('notificationLink', () => {
  it('upload_one_opens_viewer', () => {
    expect(notificationLink(notificationFixture('n'))).toEqual({
      to: '/photos/$id',
      params: { id: 'a' },
    })
  })

  it('upload_many_opens_day', () => {
    const n = notificationFixture('n', {
      count: 3,
      asset: assetSummaryFixture('a', { local_date: '2026-11-14' }),
    })
    expect(notificationLink(n)).toEqual({
      to: '/days/$date',
      params: { date: '2026-11-14' },
    })
  })

  it('photo_comment_and_mention_open_comments', () => {
    for (const kind of ['comment', 'mention'] as const) {
      const link = notificationLink(
        notificationFixture('n', { kind, comment: { excerpt: '예뻐요' } }),
      )
      expect(link).toEqual({
        to: '/photos/$id',
        params: { id: 'a' },
        search: { panel: 'comments' },
      })
    }
  })

  it('backup_failure_opens_backup_screen', () => {
    const n = notificationFixture('n', { kind: 'backup', asset: null })
    expect(notificationLink(n)).toEqual({ to: '/me/backup' })
  })

  it('story_kinds_open_story', () => {
    const cases = [
      notificationFixture('n', { kind: 'story', asset: null, story }),
      notificationFixture('n', {
        kind: 'comment',
        asset: null,
        story,
        comment: { excerpt: '좋아요' },
      }),
      notificationFixture('n', {
        kind: 'mention',
        asset: null,
        story,
        comment: { excerpt: '좋아요' },
      }),
      // 이야기 본문 멘션.
      notificationFixture('n', { kind: 'mention', asset: null, story }),
    ]
    for (const n of cases) {
      expect(notificationLink(n)).toEqual({
        to: '/stories/$id',
        params: { id: 's' },
      })
    }
  })

  it('milestone_opens_milestone', () => {
    const n = notificationFixture('n', {
      kind: 'milestone',
      asset: null,
      milestone: {
        id: 'm',
        preset_key: 'first_tooth',
        custom_label: null,
        cover: null,
      },
    })
    expect(notificationLink(n)).toEqual({
      to: '/milestones/$id',
      params: { id: 'm' },
    })
  })

  it('missing_target_has_no_link', () => {
    expect(notificationLink(notificationFixture('n', { asset: null }))).toBe(
      null,
    )
  })
})

describe('sectionOf', () => {
  const tz = 'Asia/Seoul'
  const today = '2026-11-16'

  it('today_by_instance_midnight', () => {
    // 2026-11-16 00:00 KST.
    expect(sectionOf('2026-11-15T15:00:00Z', today, tz)).toBe('today')
    // 2026-11-15 23:59 KST.
    expect(sectionOf('2026-11-15T14:59:00Z', today, tz)).toBe('week')
  })

  it('week_is_last_seven_days', () => {
    expect(sectionOf('2026-11-13T03:00:00Z', today, tz)).toBe('week')
    // 7일 전 23:59 KST 까지 이번 주.
    expect(sectionOf('2026-11-09T14:59:00Z', today, tz)).toBe('week')
    // 7일 전 00:00 KST.
    expect(sectionOf('2026-11-08T15:00:00Z', today, tz)).toBe('week')
  })

  it('earlier_from_eight_days', () => {
    expect(sectionOf('2026-11-08T03:00:00Z', today, tz)).toBe('earlier')
    // 8일 전 23:59 KST.
    expect(sectionOf('2026-11-08T14:59:00Z', today, tz)).toBe('earlier')
  })
})
