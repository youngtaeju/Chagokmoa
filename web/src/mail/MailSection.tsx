import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { api, call } from '@/api/client'
import type { components } from '@/api/schema'
import { useMe } from '@/auth/session'
import { Field, FormError, formText } from '@/components/Field'
import { SelectField } from '@/components/NativeSelect'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  MAIL_PRESETS,
  presetOf,
  type MailPresetId,
  type MailSecurity,
} from './presets'
import { mailQueryKey } from './useMail'

type MailConfig = components['schemas']['MailConfigView']

const SECURITIES: MailSecurity[] = ['starttls', 'tls', 'none']

function useSaveMail() {
  const { t } = useTranslation()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: components['schemas']['MailSettingsRequest']) =>
      call(api.PUT('/api/mail', { body })),
    onSuccess: (res) => {
      qc.setQueryData(mailQueryKey, res)
      toast(t('mail.saved'))
    },
  })
}

function ConfigForm({ config }: { config: MailConfig | null }) {
  const { t } = useTranslation()
  const save = useSaveMail()
  const [host, setHost] = useState(config?.host ?? '')
  const [port, setPort] = useState(String(config?.port ?? 587))
  const [security, setSecurity] = useState<MailSecurity>(
    config?.security ?? 'starttls',
  )
  const [password, setPassword] = useState('')
  const [preset, setPreset] = useState<MailPresetId | 'custom'>(
    config ? presetOf(config.host, config.port, config.security) : 'custom',
  )

  function edit(next: {
    host?: string
    port?: string
    security?: MailSecurity
  }) {
    const h = next.host ?? host
    let p = next.port ?? port
    const s = next.security ?? security
    // 보안 전환 시 기본 포트(587, 465)만 짝 값으로. 그 밖의 포트는 유지.
    if (next.security !== undefined && next.port === undefined) {
      if (s === 'tls' && p === '587') p = '465'
      if (s === 'starttls' && p === '465') p = '587'
    }
    if (next.host !== undefined) setHost(h)
    setPort(p)
    if (next.security !== undefined) setSecurity(s)
    setPreset(presetOf(h, Number(p), s))
  }

  return (
    <form
      className="grid gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        const form = new FormData(e.currentTarget)
        save.mutate(
          {
            host,
            port: Number(port),
            security,
            username: formText(form, 'username'),
            from_address: formText(form, 'from_address'),
            // 비우면 키 생략. 서버가 저장된 값 유지.
            ...(password ? { password } : {}),
          },
          {
            onSuccess: () => {
              setPassword('')
            },
          },
        )
      }}
    >
      <SelectField
        label={t('mail.service')}
        name="service"
        value={preset}
        onChange={(e) => {
          const id = e.target.value
          const hit = MAIL_PRESETS.find((p) => p.id === id)
          if (hit) {
            setHost(hit.host)
            setPort(String(hit.port))
            setSecurity(hit.security)
            setPreset(hit.id)
          } else {
            setPreset('custom')
          }
        }}
      >
        <option value="gmail">{t('mail.presetGmail')}</option>
        <option value="naver">{t('mail.presetNaver')}</option>
        <option value="custom">{t('mail.presetCustom')}</option>
      </SelectField>
      {preset === 'gmail' && (
        <p className="text-xs text-muted-foreground">{t('mail.hintGmail')}</p>
      )}
      {preset === 'naver' && (
        <p className="text-xs text-muted-foreground">{t('mail.hintNaver')}</p>
      )}
      <Field
        label={t('mail.host')}
        name="host"
        value={host}
        onChange={(e) => {
          edit({ host: e.target.value })
        }}
        required
        autoCapitalize="none"
        spellCheck={false}
      />
      <div className="grid grid-cols-2 gap-3">
        <Field
          label={t('mail.port')}
          name="port"
          type="number"
          inputMode="numeric"
          min={1}
          max={65535}
          value={port}
          onChange={(e) => {
            edit({ port: e.target.value })
          }}
          required
        />
        <SelectField
          label={t('mail.security')}
          name="security"
          value={security}
          onChange={(e) => {
            edit({ security: e.target.value as MailSecurity })
          }}
        >
          {SECURITIES.map((s) => (
            <option key={s} value={s}>
              {t(`mail.security_${s}`)}
            </option>
          ))}
        </SelectField>
      </div>
      {security === 'none' && (
        <p role="alert" className="text-sm text-destructive">
          {t('mail.noneWarn')}
        </p>
      )}
      <Field
        label={t('mail.username')}
        name="username"
        defaultValue={config?.username ?? ''}
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
      />
      <Field
        label={t('mail.password')}
        name="password"
        type="password"
        value={password}
        onChange={(e) => {
          setPassword(e.target.value)
        }}
        autoComplete="new-password"
        placeholder={config?.password_set ? t('mail.passwordSet') : undefined}
      />
      <Field
        label={t('mail.fromAddress')}
        name="from_address"
        type="email"
        defaultValue={config?.from_address ?? ''}
        required
      />
      <FormError error={save.error} />
      <div>
        <Button type="submit" disabled={save.isPending}>
          {t('common.save')}
        </Button>
      </div>
    </form>
  )
}

function TestForm({ disabled }: { disabled: boolean }) {
  const { t } = useTranslation()
  const { user } = useMe()
  const send = useMutation({
    mutationFn: (to: string) =>
      call(api.POST('/api/mail/test', { body: { to } })),
    onSuccess: () => {
      toast(t('mail.testSent'))
    },
  })
  return (
    <form
      className="grid gap-3 border-t border-border pt-4"
      onSubmit={(e) => {
        e.preventDefault()
        const to = formText(new FormData(e.currentTarget), 'to').trim()
        if (to) send.mutate(to)
      }}
    >
      <h2 className="text-base font-bold">{t('mail.testTitle')}</h2>
      <p className="text-xs text-muted-foreground">{t('mail.testHint')}</p>
      <Field
        label={t('mail.testTo')}
        name="to"
        type="email"
        defaultValue={user?.email ?? ''}
        required
      />
      <FormError error={send.error} />
      <div>
        <Button
          type="submit"
          variant="outline"
          disabled={disabled || send.isPending}
        >
          {t('mail.testSend')}
        </Button>
      </div>
    </form>
  )
}

function DeleteSection() {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const remove = useMutation({
    mutationFn: () => call(api.DELETE('/api/mail')),
    onSuccess: () => {
      qc.setQueryData(mailQueryKey, { config: null })
      setConfirming(false)
      toast(t('mail.deleted'))
    },
  })
  function close() {
    setConfirming(false)
    remove.reset()
  }
  return (
    <section className="grid gap-3 border-t border-border pt-4">
      <div>
        <Button
          variant="outline"
          className="text-destructive"
          onClick={() => {
            setConfirming(true)
          }}
        >
          {t('mail.delete')}
        </Button>
      </div>
      <Dialog
        open={confirming}
        onOpenChange={(open) => {
          if (!open) close()
        }}
      >
        <DialogContent role="alertdialog">
          <DialogHeader>
            <DialogTitle>{t('mail.deleteTitle')}</DialogTitle>
            <DialogDescription>{t('mail.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <FormError error={remove.error} />
          <DialogFooter>
            <Button variant="outline" onClick={close}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => {
                remove.mutate()
              }}
            >
              {t('mail.deleteAction')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  )
}

export function MailSection({ config }: { config: MailConfig | null }) {
  return (
    <div className="grid gap-6">
      {/* 저장, 삭제 뒤 폼 상태 재초기화. */}
      <ConfigForm key={JSON.stringify(config)} config={config} />
      <TestForm disabled={config === null} />
      {config && <DeleteSection />}
    </div>
  )
}
