import {
  cleanup,
  fireEvent,
  screen,
  waitFor,
  within,
} from '@testing-library/react'
import {
  afterAll,
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from 'vitest'
import type { BackupRun, BackupStatus } from '@/api/types'
import { initI18n } from '@/i18n'
import ko from '@/locales/ko.json'
import { json, mockFetch } from '@/test/fetch'
import {
  apiError,
  backupStatusFixture,
  emptyFeedRoutes,
  settingsFixture,
  userFixture,
} from '@/test/fixtures'
import { renderApp } from '@/test/router'

beforeEach(() => {
  initI18n('ko')
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

// sonner 는 닫힌 토스트를 200ms(TIME_BEFORE_UNMOUNT) 뒤 setState 로 제거. 언마운트 뒤에도 타이머 잔존.
// 지금 백업 폴링 테스트가 토스트 수명(4초) 무렵 끝나면 jsdom 해제 뒤 실행돼 window 오류.
// 언마운트로 새 타이머는 안 생기므로 해제 전 한 번 소진.
afterAll(async () => {
  await new Promise((resolve) => setTimeout(resolve, 250))
})

const KEY = 'ABCD-EFGH-JKMN-PQRS-TVWX-YZ01-2345-6789'

const CONFIGURED = backupStatusFixture({
  key_set: true,
  enabled: true,
  repo_state: 'restic',
  next_run: '2026-11-19T18:00:00Z',
})

function run(over: Partial<BackupRun> = {}): BackupRun {
  return {
    id: '00000000-0000-4000-8000-0000000000f1',
    trigger: 'scheduled',
    status: 'success',
    phase: null,
    started_at: '2026-11-18T18:00:00Z',
    finished_at: '2026-11-18T18:01:05Z',
    local: {
      snapshot_id: 'abc123',
      files_new: 12,
      data_added: 2_500_000,
      duration_ms: 65_000,
    },
    remote: null,
    error: null,
    ...over,
  }
}

// 서버처럼 상태 유지. 핸들러가 state.current 를 바꿈.
function renderBackup(
  initial: BackupStatus,
  extra: (state: {
    current: BackupStatus
  }) => Parameters<typeof mockFetch>[0] = () => ({}),
) {
  const state = { current: initial, gets: 0 }
  mockFetch({
    ...emptyFeedRoutes,
    'GET /api/auth/me': () => json(200, { user: userFixture('owner') }),
    'GET /api/settings': () => json(200, settingsFixture),
    'GET /api/derivatives/regenerate': () =>
      json(200, { remaining: 0, failed: 0 }),
    'GET /api/backup': () => {
      state.gets += 1
      return json(200, state.current)
    },
    ...extra(state),
  })
  return { ...renderApp('/settings/backup'), state }
}

describe('backup setup', () => {
  it('not_mounted_shows_compose_guidance', async () => {
    renderBackup(backupStatusFixture({ mounted: false, repo_state: null }))
    expect(await screen.findByText(ko.backup.mountTitle)).toBeInTheDocument()
    expect(screen.getByText(ko.backup.mountOwner)).toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: ko.backup.createKey }),
    ).toBeNull()
    expect(screen.queryByRole('button', { name: ko.backup.enable })).toBeNull()
  })

  it('empty_repo_key_copy_download_confirm_then_enable', async () => {
    let enableBody: unknown = null
    const { user } = renderBackup(backupStatusFixture(), (s) => ({
      'POST /api/backup/key': () => json(200, { key: KEY }),
      'POST /api/backup/enable': async (req) => {
        enableBody = await req.json()
        s.current = CONFIGURED
        return json(200, s.current)
      },
    }))
    await user.click(
      await screen.findByRole('button', { name: ko.backup.createKey }),
    )
    expect(await screen.findByText(KEY)).toBeInTheDocument()
    expect(screen.getByText(ko.backup.keyWarn)).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: ko.common.copy }))
    await waitFor(async () => {
      expect(await navigator.clipboard.readText()).toBe(KEY)
    })
    expect(await screen.findByText(ko.common.copied)).toBeInTheDocument()

    const download = screen.getByRole('link', { name: ko.backup.keyDownload })
    expect(download).toHaveAttribute('download', 'chagokmoa-recovery-key.txt')
    expect(decodeURIComponent(download.getAttribute('href') ?? '')).toContain(
      KEY,
    )

    const enable = screen.getByRole('button', { name: ko.backup.enable })
    expect(enable).toBeDisabled()
    await user.click(screen.getByRole('checkbox', { name: ko.backup.keySaved }))
    expect(enable).toBeEnabled()
    await user.click(enable)

    expect(
      await screen.findByRole('button', { name: ko.backup.runNow }),
    ).toBeInTheDocument()
    expect(enableBody).toEqual({ key: KEY, confirmed: true })
    expect(screen.getByText(ko.backup.never)).toBeInTheDocument()
    expect(screen.queryByText(KEY)).toBeNull()
  })

  it('existing_repo_trims_key_and_shows_wrong_key', async () => {
    const bodies: unknown[] = []
    const { user } = renderBackup(
      backupStatusFixture({ repo_state: 'restic' }),
      (s) => ({
        'POST /api/backup/enable': async (req) => {
          bodies.push(await req.json())
          if (bodies.length === 1) return apiError(409, 'backup.wrongKey')
          s.current = CONFIGURED
          return json(200, s.current)
        },
      }),
    )
    expect(await screen.findByText(ko.backup.existingRestore)).toBeVisible()
    expect(
      screen.queryByRole('button', { name: ko.backup.createKey }),
    ).toBeNull()
    await user.type(screen.getByLabelText(ko.backup.existingKey), '  OLD-KEY  ')
    await user.click(screen.getByRole('button', { name: ko.backup.enable }))
    expect(
      await screen.findByText(ko.errors.backup.wrongKey),
    ).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: ko.backup.enable }))
    expect(
      await screen.findByRole('button', { name: ko.backup.runNow }),
    ).toBeInTheDocument()
    expect(bodies).toEqual([{ key: 'OLD-KEY' }, { key: 'OLD-KEY' }])
  })

  it('foreign_repo_refused', async () => {
    renderBackup(backupStatusFixture({ repo_state: 'foreign' }))
    expect(await screen.findByText(ko.backup.foreign)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: ko.backup.enable })).toBeNull()
    expect(
      screen.queryByRole('button', { name: ko.backup.createKey }),
    ).toBeNull()
  })

  it('enable_race_loser_refreshes_state', async () => {
    const { user, state } = renderBackup(backupStatusFixture(), (s) => ({
      'POST /api/backup/key': () => json(200, { key: KEY }),
      'POST /api/backup/enable': () => {
        // 다른 요청이 먼저 켬.
        s.current = CONFIGURED
        return apiError(409, 'backup.alreadyConfigured')
      },
    }))
    await user.click(
      await screen.findByRole('button', { name: ko.backup.createKey }),
    )
    await user.click(
      await screen.findByRole('checkbox', { name: ko.backup.keySaved }),
    )
    const before = state.gets
    await user.click(screen.getByRole('button', { name: ko.backup.enable }))
    expect(
      await screen.findByRole('button', { name: ko.backup.runNow }),
    ).toBeInTheDocument()
    expect(state.gets).toBeGreaterThan(before)
  })
})

describe('backup settings', () => {
  it('toggle_and_save_time_retention', async () => {
    const bodies: unknown[] = []
    const { user } = renderBackup(CONFIGURED, (s) => ({
      'PUT /api/backup': async (req) => {
        const body = (await req.json()) as Partial<BackupStatus>
        bodies.push(body)
        s.current = { ...s.current, ...body }
        return json(200, s.current)
      },
    }))
    const sw = await screen.findByRole('switch', { name: ko.backup.autoToggle })
    expect(sw).toHaveAttribute('aria-checked', 'true')
    await user.click(sw)
    await waitFor(() => {
      expect(sw).toHaveAttribute('aria-checked', 'false')
    })
    expect(bodies).toEqual([{ enabled: false }])

    fireEvent.change(screen.getByLabelText(ko.backup.time), {
      target: { value: '04:30' },
    })
    const daily = screen.getByLabelText(ko.backup.daily)
    await user.clear(daily)
    await user.type(daily, '14')
    const region = screen.getByRole('region', { name: ko.backup.settingsTitle })
    await user.click(
      within(region).getByRole('button', { name: ko.common.save }),
    )
    expect(await screen.findByText(ko.backup.settingsSaved)).toBeInTheDocument()
    expect(bodies[1]).toEqual({
      time: '04:30',
      retention: { daily: 14, weekly: 4, monthly: 12 },
    })
  })

  it('reveal_key_needs_password', async () => {
    const bodies: unknown[] = []
    const { user } = renderBackup(CONFIGURED, () => ({
      'POST /api/backup/key/reveal': async (req) => {
        const body = (await req.json()) as { password: string }
        bodies.push(body)
        return body.password === 'right-pass'
          ? json(200, { key: KEY })
          : apiError(401, 'auth.invalidCredentials')
      },
    }))
    await user.click(
      await screen.findByRole('button', { name: ko.backup.keyReveal }),
    )
    const dialog = await screen.findByRole('dialog')
    const password = within(dialog).getByLabelText(ko.backup.password)
    await user.type(password, 'wrong')
    await user.click(
      within(dialog).getByRole('button', { name: ko.backup.keyShow }),
    )
    expect(
      await within(dialog).findByText(ko.backup.revealWrong),
    ).toBeInTheDocument()
    await user.clear(password)
    await user.type(password, 'right-pass')
    await user.click(
      within(dialog).getByRole('button', { name: ko.backup.keyShow }),
    )
    expect(await within(dialog).findByText(KEY)).toBeInTheDocument()
    expect(bodies).toEqual([{ password: 'wrong' }, { password: 'right-pass' }])
  })
})

describe('backup remote', () => {
  it('save_requires_passing_test_of_same_values', async () => {
    const tests: unknown[] = []
    const saves: unknown[] = []
    let release: () => void = () => undefined
    const { user } = renderBackup(CONFIGURED, (s) => ({
      'POST /api/backup/remote/test': async (req) => {
        tests.push(await req.json())
        if (tests.length === 1) {
          return new Promise((resolve) => {
            release = () => {
              resolve(
                json(200, {
                  ok: false,
                  state: null,
                  message: 'dial tcp: i/o timeout',
                }),
              )
            }
          })
        }
        return json(200, { ok: true, state: 'empty', message: null })
      },
      'PUT /api/backup/remote': async (req) => {
        const body = (await req.json()) as { repository: string }
        saves.push(body)
        s.current = {
          ...s.current,
          remote: { repository: body.repository, configured: true },
        }
        return json(200, s.current)
      },
    }))
    await user.click(
      await screen.findByRole('button', { name: ko.backup.remoteAdd }),
    )
    await user.type(
      screen.getByLabelText(ko.backup.remoteRepository),
      's3:https://s3.example.com/bucket',
    )
    await user.type(screen.getByLabelText('AWS_ACCESS_KEY_ID'), 'id')
    await user.type(screen.getByLabelText('AWS_SECRET_ACCESS_KEY'), 'secret')
    const region = screen.getByRole('region', { name: ko.backup.remoteTitle })
    const save = within(region).getByRole('button', { name: ko.common.save })
    const test = within(region).getByRole('button', {
      name: ko.backup.remoteTest,
    })
    expect(save).toBeDisabled()

    await user.click(test)
    expect(await screen.findByText(ko.backup.remoteTesting)).toBeVisible()
    expect(test).toBeDisabled()
    release()
    expect(
      await screen.findByText(ko.backup.remoteTestFailed),
    ).toBeInTheDocument()
    expect(screen.getByText('dial tcp: i/o timeout')).toBeInTheDocument()
    expect(save).toBeDisabled()

    await user.click(test)
    expect(
      await screen.findByText(ko.backup.remoteTestEmpty),
    ).toBeInTheDocument()
    expect(save).toBeEnabled()

    // 시험 뒤 값이 바뀌면 다시 시험.
    await user.type(screen.getByLabelText('AWS_SECRET_ACCESS_KEY'), '2')
    expect(save).toBeDisabled()
    expect(screen.queryByText(ko.backup.remoteTestEmpty)).toBeNull()
    await user.click(test)
    await waitFor(() => {
      expect(save).toBeEnabled()
    })
    await user.click(save)

    expect(await screen.findByText(ko.backup.remoteSaved)).toBeInTheDocument()
    const body = {
      repository: 's3:https://s3.example.com/bucket',
      env: { AWS_ACCESS_KEY_ID: 'id', AWS_SECRET_ACCESS_KEY: 'secret2' },
    }
    expect(tests[2]).toEqual(body)
    expect(saves).toEqual([body])
    expect(screen.getByText(ko.backup.remoteCreds)).toBeInTheDocument()
    expect(screen.queryByLabelText(ko.backup.remoteRepository)).toBeNull()
  })

  it('masked_remote_not_prefilled_and_delete', async () => {
    let deleted = false
    const { user } = renderBackup(
      {
        ...CONFIGURED,
        remote: { repository: 'rest:https://u:***@h/r', configured: false },
      },
      (s) => ({
        'DELETE /api/backup/remote': () => {
          deleted = true
          s.current = { ...s.current, remote: null }
          return json(204, null)
        },
      }),
    )
    expect(
      await screen.findByText('rest:https://u:***@h/r'),
    ).toBeInTheDocument()
    expect(screen.getByText(ko.backup.remoteUrlCreds)).toBeInTheDocument()

    await user.click(
      screen.getByRole('button', { name: ko.backup.remoteChange }),
    )
    expect(screen.getByLabelText(ko.backup.remoteRepository)).toHaveValue('')
    expect(screen.getByText(ko.backup.remoteReenter)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: ko.common.cancel }))

    await user.click(
      screen.getByRole('button', { name: ko.backup.remoteDelete }),
    )
    const dialog = await screen.findByRole('alertdialog')
    await user.click(
      within(dialog).getByRole('button', { name: ko.common.delete }),
    )
    expect(await screen.findByText(ko.backup.remoteNone)).toBeInTheDocument()
    expect(deleted).toBe(true)
  })
})

describe('backup status', () => {
  beforeEach(() => {
    // 2026-11-19 09:00 KST. 타이머는 실제.
    vi.useFakeTimers({
      toFake: ['Date'],
      now: new Date('2026-11-19T00:00:00Z'),
    })
  })

  it('shows_last_next_and_runs', async () => {
    const last = run({
      status: 'warning',
      local: {
        snapshot_id: 'abc123',
        files_new: 12,
        data_added: 2_500_000,
        duration_ms: 65_000,
        warning: 'local repository created; 2 files vanished during backup',
      },
      remote: { error: 'connection refused' },
    })
    const older = run({
      id: '00000000-0000-4000-8000-0000000000f0',
      status: 'failed',
      trigger: 'manual',
      started_at: '2026-11-17T18:00:00Z',
      finished_at: null,
      local: null,
      error: 'interrupted',
    })
    renderBackup({ ...CONFIGURED, last_run: last, runs: [last, older] })
    const status = await screen.findByRole('region', {
      name: ko.backup.statusTitle,
    })
    expect(within(status).getByText(ko.backup.warnLocalCreated)).toBeVisible()
    expect(
      within(status).getByText('백업하는 동안 지워진 파일 2개는 빠졌어요.'),
    ).toBeVisible()
    expect(within(status).getByText('connection refused')).toBeVisible()
    expect(within(status).getByText(/새 파일 12개/)).toBeVisible()
    expect(within(status).getByText(/11월 20일/)).toBeVisible()

    const runs = within(status).getByRole('list', { name: ko.backup.runsTitle })
    const items = within(runs).getAllByRole('listitem')
    expect(items).toHaveLength(2)
    expect(items[1]).toHaveTextContent(ko.backup.status.failed)
    expect(items[1]).toHaveTextContent(ko.backup.trigger.manual)
    expect(items[1]).toHaveTextContent(ko.backup.interrupted)
  })

  it('failed_before_restic_shows_error_without_repo_lines', async () => {
    const last = run({
      status: 'failed',
      local: {},
      remote: {},
      error: 'dump: pg_dump failed',
    })
    renderBackup({ ...CONFIGURED, last_run: last, runs: [last] })
    const status = await screen.findByRole('region', {
      name: ko.backup.statusTitle,
    })
    expect(within(status).queryByText(/새 파일/)).toBeNull()
    expect(within(status).queryByText(ko.backup.local)).toBeNull()
    expect(within(status).queryByText(ko.backup.remote)).toBeNull()
    expect(
      within(status).getAllByText(/pg_dump failed/).length,
    ).toBeGreaterThan(0)
  })

  it('run_now_disabled_when_not_mounted', async () => {
    renderBackup({ ...CONFIGURED, mounted: false, repo_state: null })
    expect(
      await screen.findByRole('button', { name: ko.backup.runNow }),
    ).toBeDisabled()
    expect(screen.getByText(ko.backup.mountTitle)).toBeInTheDocument()
  })

  it('past_next_run_is_soon_and_unmounted_shows_guidance', async () => {
    renderBackup({ ...CONFIGURED, next_run: '2026-11-18T18:00:00Z' })
    expect(await screen.findByText(ko.backup.nextSoon)).toBeInTheDocument()
    cleanup()

    renderBackup({
      ...CONFIGURED,
      mounted: false,
      repo_state: null,
      next_run: '2026-11-18T18:00:00Z',
    })
    expect(await screen.findByText(ko.backup.mountTitle)).toBeInTheDocument()
    expect(screen.queryByText(ko.backup.nextSoon)).toBeNull()
    expect(screen.queryByText(ko.backup.nextRun)).toBeNull()
  })

  it('run_now_disables_and_polls_until_done', async () => {
    let posted = 0
    let polls = 0
    const running = run({
      status: 'running',
      phase: 'local',
      trigger: 'manual',
      finished_at: null,
      local: null,
    })
    const done = run({ trigger: 'manual' })
    const { user } = renderBackup(CONFIGURED, (s) => ({
      'POST /api/backup/run': () => {
        posted += 1
        s.current = { ...CONFIGURED, runs: [running] }
        return json(202, { run_id: running.id })
      },
      'GET /api/backup': () => {
        if (s.current.runs[0]?.status === 'running') {
          polls += 1
          if (polls > 2)
            s.current = { ...CONFIGURED, last_run: done, runs: [done] }
        }
        return json(200, s.current)
      },
    }))
    const button = await screen.findByRole('button', { name: ko.backup.runNow })
    await user.click(button)
    expect(await screen.findByText(ko.backup.running)).toBeInTheDocument()
    expect(screen.getByText(ko.backup.phase.local)).toBeInTheDocument()
    expect(button).toBeDisabled()
    await waitFor(
      () => {
        expect(screen.queryByText(ko.backup.running)).toBeNull()
      },
      { timeout: 8000 },
    )
    expect(button).toBeEnabled()
    expect(posted).toBe(1)
    expect(polls).toBeGreaterThan(2)
  }, 15_000)

  it('run_now_conflict_running_switches_to_progress', async () => {
    const running = run({
      status: 'running',
      phase: 'dump',
      finished_at: null,
      local: null,
    })
    const { user } = renderBackup(CONFIGURED, (s) => ({
      'POST /api/backup/run': () => {
        // 일정 실행이 먼저 시작.
        s.current = { ...CONFIGURED, runs: [running] }
        return apiError(409, 'backup.running')
      },
    }))
    await user.click(
      await screen.findByRole('button', { name: ko.backup.runNow }),
    )
    expect(await screen.findByText(ko.backup.phase.dump)).toBeInTheDocument()
    expect(screen.queryByText(ko.errors.backup.running)).toBeNull()
  })
})
