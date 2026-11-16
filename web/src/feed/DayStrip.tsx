import { useQueries, type UseQueryResult } from '@tanstack/react-query'
import { useNavigate, useRouter } from '@tanstack/react-router'
import {
  type KeyboardEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { useTranslation } from 'react-i18next'
import IconPrev from '~icons/material-symbols/chevron-left-rounded'
import IconNext from '~icons/material-symbols/chevron-right-rounded'
import type { DayPage as DaysPage } from '@/api/types'
import { addMonths, monthRange } from '@/calendar/month'
import { monthQuery } from '@/calendar/useMonth'
import { Button } from '@/components/ui/button'
import {
  addDays,
  formatDayTitle,
  formatMonthShort,
  formatWeekday,
} from '@/lib/dates'
import { cn } from '@/lib/utils'
import { useToday } from '@/settings/useSettings'

type Item = { kind: 'month'; month: string } | { kind: 'day'; date: string }

// 처음 그릴 범위: 지금 날 달 앞뒤 한 달.
function around(month: string) {
  return { from: addMonths(month, -1), to: addMonths(month, 1) }
}

// 띠 끝에서 이만큼 안쪽이 보이면 다음 달 그림.
const EDGE_MARGIN = '0px 600px'

// 부드러운 스크롤 목표 유지 상한. scrollend 미지원 브라우저 대비.
const GLIDE_MS = 1000

// 모듈 수준 함수. 결과 같으면 useQueries 가 이전 Set 재사용.
function recordDates(results: UseQueryResult<DaysPage>[]): Set<string> {
  const dates = new Set<string>()
  for (const r of results)
    for (const d of r.data?.items ?? []) dates.add(d.date)
  return dates
}

function smooth(): ScrollBehavior {
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches
    ? 'auto'
    : 'smooth'
}

// 그날 머리글 아래 가로 날짜 띠. first ~ last 모든 날, 기록 여부는 달 단위 조회(달력과 같은 캐시).
// 지금 날은 가운데, 다른 날 이동은 기록 대체(뒤로 가기 = 들어오기 전 화면).
export function DayStrip({
  date,
  first,
  last,
  baby,
}: {
  date: string
  first: string
  last: string
  baby?: string | undefined
}) {
  const { t, i18n } = useTranslation()
  const lng = i18n.language
  const today = useToday()
  const firstMonth = first.slice(0, 7)
  const lastMonth = last.slice(0, 7)
  const dateMonth = date.slice(0, 7)

  // 범위 기준 달. first ~ last 밖 날(범위 응답 갱신 전 잠깐)이면 가까운 끝 달.
  const anchor =
    dateMonth < firstMonth
      ? firstMonth
      : dateMonth > lastMonth
        ? lastMonth
        : dateMonth
  const [range, setRange] = useState(() => around(anchor))
  // 띠 밖 날로 이동(알림 등)하면 그 날 기준으로 다시.
  if (anchor < range.from || anchor > range.to) {
    setRange(around(anchor))
  }
  const from = range.from > firstMonth ? range.from : firstMonth
  const to = range.to < lastMonth ? range.to : lastMonth
  const moreBefore = from > firstMonth
  const moreAfter = to < lastMonth

  const months = useMemo(() => {
    const out: string[] = []
    for (let m = from; m <= to; m = addMonths(m, 1)) out.push(m)
    return out
  }, [from, to])
  const records = useQueries({
    queries: months.map((m) => monthQuery(m, baby)),
    combine: recordDates,
  })

  const items = useMemo(() => {
    const out: Item[] = []
    const start = `${from}-01` > first ? `${from}-01` : first
    const end = monthRange(to).to < last ? monthRange(to).to : last
    for (let d = start; d <= end; d = addDays(d, 1)) {
      if (d === start || d.endsWith('-01')) {
        out.push({ kind: 'month', month: d.slice(0, 7) })
      }
      out.push({ kind: 'day', date: d })
    }
    return out
  }, [from, to, first, last])

  // 띠 들어오는 Tab 자리 하나. 지금 날이 범위 밖이면 가까운 끝 기록 날.
  const tabDate = useMemo(() => {
    if (date >= first && date <= last) return date
    const days = items.flatMap((i) =>
      i.kind === 'day' && records.has(i.date) ? [i.date] : [],
    )
    return date < first ? days[0] : days.at(-1)
  }, [date, first, last, items, records])

  const scroller = useRef<HTMLDivElement>(null)
  const startEdge = useRef<HTMLSpanElement>(null)
  const endEdge = useRef<HTMLSpanElement>(null)

  // 진행 중 부드러운 스크롤 목표. 칸 위치 보정 뒤 이어 가기용.
  const smoothTarget = useRef<number | null>(null)
  const glideTimer = useRef<number | undefined>(undefined)
  const glide = useCallback((left: number, behavior: ScrollBehavior) => {
    const el = scroller.current
    if (!el) return
    el.scrollTo({ left, behavior })
    window.clearTimeout(glideTimer.current)
    smoothTarget.current = null
    if (behavior !== 'smooth') return
    smoothTarget.current = left
    glideTimer.current = window.setTimeout(() => {
      smoothTarget.current = null
    }, GLIDE_MS)
  }, [])

  // 앞쪽 칸 수가 바뀌면(앞 달 덧붙임, 범위 재조회) 지금 날 칸 화면 위치 유지.
  // 진행 중 부드러운 스크롤은 같은 거리만큼 옮긴 목표로 이어 감.
  // 기준은 마지막 스크롤 이벤트 위치. 멈춘 띠는 스냅 재정렬이 레이아웃 때 scrollLeft 를 먼저 옮김.
  const kept = useRef<{ date: string; left: number } | null>(null)
  const lastLeft = useRef(0)
  useLayoutEffect(() => {
    const el = scroller.current
    const cell = el?.querySelector<HTMLElement>(`[data-date="${date}"]`)
    const prev = kept.current
    kept.current = cell ? { date, left: cell.offsetLeft } : null
    if (!el || !cell || prev?.date !== date) return
    const d = cell.offsetLeft - prev.left
    if (d === 0) return
    el.scrollLeft = lastLeft.current + d
    lastLeft.current = el.scrollLeft
    if (smoothTarget.current !== null) glide(smoothTarget.current + d, 'smooth')
  }, [date, items, glide])

  // 처음은 바로, 이후 날 바뀌면 부드럽게 지금 날 칸을 가운데로.
  // 라우터 스크롤 복원이 이동 직후 띠 scrollLeft 를 이전 값으로 되돌림. 다음 프레임에 다시 맞춤.
  // 완료 표시는 프레임 안에서. 그 전에 다시 그리면 취소 후 재예약.
  const centered = useRef<string | null>(null)
  useLayoutEffect(() => {
    const el = scroller.current
    if (!el || centered.current === date) return
    const cell = el.querySelector<HTMLElement>(`[data-date="${date}"]`)
    if (!cell) return
    const left = cell.offsetLeft - (el.clientWidth - cell.offsetWidth) / 2
    const initial = centered.current === null
    if (initial) el.scrollLeft = left
    const id = requestAnimationFrame(() => {
      glide(left, initial ? 'auto' : smooth())
      centered.current = date
    })
    return () => {
      cancelAnimationFrame(id)
    }
  }, [date, items, glide])

  useEffect(() => {
    const el = scroller.current
    if (!el || (!moreBefore && !moreAfter)) return
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue
          if (e.target === startEdge.current) {
            setRange((r) => ({ ...r, from: addMonths(from, -1) }))
          } else {
            setRange((r) => ({ ...r, to: addMonths(to, 1) }))
          }
        }
      },
      { root: el, rootMargin: EDGE_MARGIN },
    )
    if (moreBefore && startEdge.current) io.observe(startEdge.current)
    if (moreAfter && endEdge.current) io.observe(endEdge.current)
    return () => {
      io.disconnect()
    }
  }, [from, to, moreBefore, moreAfter])

  // ‹ › 비활성: 그쪽으로 더 밀 수 없음.
  const [edges, setEdges] = useState({ atStart: true, atEnd: true })
  const measure = useCallback(() => {
    const el = scroller.current
    if (!el) return
    const atStart = el.scrollLeft <= 1
    const atEnd = el.scrollLeft + el.clientWidth >= el.scrollWidth - 1
    setEdges((prev) =>
      prev.atStart === atStart && prev.atEnd === atEnd
        ? prev
        : { atStart, atEnd },
    )
  }, [])
  useLayoutEffect(measure, [measure, items])
  useEffect(() => {
    window.addEventListener('resize', measure)
    return () => {
      window.removeEventListener('resize', measure)
    }
  }, [measure])

  const page = (dir: -1 | 1) => {
    const el = scroller.current
    if (el) glide(el.scrollLeft + dir * el.clientWidth, smooth())
  }

  return (
    <nav aria-label={t('day.nav')} className="flex items-center pb-1 md:px-1">
      <Button
        variant="ghost"
        size="icon"
        aria-label={t('day.stripEarlier')}
        disabled={edges.atStart}
        onClick={() => {
          page(-1)
        }}
        className="hidden md:inline-flex"
      >
        <IconPrev aria-hidden className="size-5" />
      </Button>
      <div
        ref={scroller}
        onScroll={(e) => {
          lastLeft.current = e.currentTarget.scrollLeft
          measure()
        }}
        onScrollEnd={() => {
          smoothTarget.current = null
        }}
        className="relative flex min-w-0 flex-1 snap-x snap-proximity gap-1.5 overflow-x-auto px-3 py-1 [overflow-anchor:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {moreBefore && (
          <span ref={startEdge} aria-hidden className="w-px shrink-0" />
        )}
        {items.map((item) =>
          item.kind === 'month' ? (
            <span
              key={item.month}
              aria-hidden
              className="shrink-0 self-center rounded-full bg-accent-soft px-2 py-1 text-xs font-extrabold whitespace-nowrap text-accent-ink"
            >
              {/* 연도 바뀌는 1월은 연도까지. */}
              {formatMonthShort(item.month, lng, item.month.endsWith('-01'))}
            </span>
          ) : (
            <DayCell
              key={item.date}
              date={item.date}
              label={formatDayTitle(item.date, today, lng)}
              weekday={formatWeekday(item.date, lng)}
              state={
                item.date === date
                  ? 'current'
                  : records.has(item.date)
                    ? 'record'
                    : 'empty'
              }
              tabbable={item.date === tabDate}
              baby={baby}
            />
          ),
        )}
        {moreAfter && (
          <span ref={endEdge} aria-hidden className="w-px shrink-0" />
        )}
      </div>
      <Button
        variant="ghost"
        size="icon"
        aria-label={t('day.stripLater')}
        disabled={edges.atEnd}
        onClick={() => {
          page(1)
        }}
        className="hidden md:inline-flex"
      >
        <IconNext aria-hidden className="size-5" />
      </Button>
    </nav>
  )
}

// 띠 안 화살표, Home, End: 기록 날 링크 사이 포커스 이동(빈 날은 링크 아님).
// 포커스 기본 스크롤은 일부 가려진 칸을 그대로 두고, 가장자리 맞춤은 스냅이 다시 가림. 벗어난 칸은 가운데로.
function moveFocus(e: KeyboardEvent<HTMLAnchorElement>) {
  if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return
  // Alt+화살표 등 브라우저 단축키는 그대로.
  if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return
  const strip = e.currentTarget.parentElement
  if (!strip) return
  e.preventDefault()
  const links = Array.from(
    strip.querySelectorAll<HTMLAnchorElement>('a[data-date]'),
  )
  const i = links.indexOf(e.currentTarget)
  const next =
    e.key === 'Home'
      ? links[0]
      : e.key === 'End'
        ? links.at(-1)
        : links[e.key === 'ArrowLeft' ? i - 1 : i + 1]
  if (!next) return
  next.focus({ preventScroll: true })
  const box = strip.getBoundingClientRect()
  const cell = next.getBoundingClientRect()
  if (cell.left < box.left || cell.right > box.right) {
    next.scrollIntoView({ block: 'nearest', inline: 'center' })
  }
}

// 칸: 요일 + 일 + 기록 점. 기록 없는 날은 흐리게, 링크 아님, 스크린 리더 숨김.
// 지금 날과 기록 있는 날은 같은 a 요소(이동 후에도 키보드 포커스 유지).
// 라우터 Link 는 활성 링크에 aria-current="page" 를 덮어써 직접 a + navigate.
function DayCell({
  date,
  label,
  weekday,
  state,
  tabbable,
  baby,
}: {
  date: string
  label: string
  weekday: string
  state: 'current' | 'record' | 'empty'
  tabbable: boolean
  baby: string | undefined
}) {
  const router = useRouter()
  const navigate = useNavigate()
  const className = cn(
    'flex w-10 shrink-0 snap-center flex-col items-center gap-0.5 rounded-xl pt-1.5 pb-2 tabular-nums focus-visible:-outline-offset-2',
    state === 'current' && 'bg-primary text-primary-foreground',
    state === 'record' && 'hover:bg-muted',
    state === 'empty' && 'text-muted-foreground/50',
  )
  const body = (
    <>
      <span
        className={cn(
          'text-2xs',
          state === 'record' && 'text-muted-foreground',
        )}
      >
        {weekday}
      </span>
      <span className="text-base leading-5 font-bold">
        {Number(date.slice(8))}
      </span>
      <span
        className={cn(
          'size-1 rounded-full',
          state === 'current' && 'bg-primary-foreground',
          state === 'record' && 'bg-primary',
        )}
      />
    </>
  )
  if (state === 'empty') {
    return (
      <span aria-hidden data-date={date} className={className}>
        {body}
      </span>
    )
  }
  const href = router.history.createHref(
    router.buildLocation({
      to: '/days/$date',
      params: { date },
      search: { baby },
    }).publicHref,
  )
  return (
    <a
      href={href}
      aria-label={label}
      aria-current={state === 'current' ? 'date' : undefined}
      data-date={date}
      // 띠 Tab 자리 하나(로빙 tabindex). 나머지 칸은 화살표로.
      tabIndex={tabbable ? 0 : -1}
      onKeyDown={moveFocus}
      onClick={(e) => {
        // 수정 키, 가운데 버튼은 브라우저 기본(새 탭 등).
        if (
          e.defaultPrevented ||
          e.button !== 0 ||
          e.metaKey ||
          e.ctrlKey ||
          e.shiftKey ||
          e.altKey
        ) {
          return
        }
        e.preventDefault()
        if (state === 'current') return
        void navigate({
          to: '/days/$date',
          params: { date },
          search: { baby },
          replace: true,
        })
      }}
      className={className}
    >
      {body}
    </a>
  )
}
