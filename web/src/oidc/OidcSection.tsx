import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useId, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { api, call } from '@/api/client'
import type { components } from '@/api/schema'
import { Field, FormError, formText } from '@/components/Field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { oidcQueryKey } from './useOidc'

type OidcResponse = components['schemas']['OidcSettingsResponse']
type OidcRequest = components['schemas']['OidcSettingsRequest']

function useSaveOidc() {
  const { t } = useTranslation()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: OidcRequest) => call(api.PUT('/api/oidc', { body })),
    onSuccess: (res) => {
      qc.setQueryData(oidcQueryKey, res)
      toast(t('oidc.saved'))
    },
  })
}

// 읽기 전용 주소와 복사. 폼 안이라 버튼은 type="button".
function CallbackAddress({ url }: { url: string }) {
  const { t } = useTranslation()
  const id = useId()
  const input = useRef<HTMLInputElement>(null)
  const [copyFailed, setCopyFailed] = useState(false)

  async function copy() {
    try {
      await navigator.clipboard.writeText(url)
      setCopyFailed(false)
      toast(t('common.copied'))
    } catch {
      setCopyFailed(true)
      input.current?.select()
    }
  }

  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{t('oidc.callbackUrl')}</Label>
      <div className="flex gap-2">
        <Input
          id={id}
          ref={input}
          readOnly
          value={url}
          onFocus={(e) => {
            e.currentTarget.select()
          }}
        />
        <Button type="button" variant="outline" onClick={() => void copy()}>
          {t('common.copy')}
        </Button>
      </div>
      {copyFailed && (
        <p role="alert" className="text-sm text-destructive">
          {t('members.copyFailed')}
        </p>
      )}
    </div>
  )
}

// 서버 same_issuer 와 같은 규칙. 끝 슬래시 하나 차이는 같은 issuer.
function sameIssuer(a: string, b: string): boolean {
  const strip = (v: string) => (v.endsWith('/') ? v.slice(0, -1) : v)
  return strip(a) === strip(b)
}

function ConfigForm({ res }: { res: OidcResponse }) {
  const { t } = useTranslation()
  const save = useSaveOidc()
  const config = res.config
  const [issuer, setIssuer] = useState(config?.issuer_url ?? '')
  const [secret, setSecret] = useState('')
  const [clearSecret, setClearSecret] = useState(false)
  // issuer 변경 확인 대기 중인 저장 본문.
  const [pending, setPending] = useState<OidcRequest | null>(null)

  function submit(body: OidcRequest) {
    save.mutate(body, {
      onSuccess: () => {
        setSecret('')
        setClearSecret(false)
        setPending(null)
      },
    })
  }

  return (
    <form
      className="grid gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        const form = new FormData(e.currentTarget)
        const body: OidcRequest = {
          issuer_url: issuer.trim(),
          client_id: formText(form, 'client_id').trim(),
          button_label: formText(form, 'button_label').trim(),
          // 지우기는 빈 문자열, 비었거나 공백뿐이면 키 생략(서버가 저장된 값 유지).
          ...(clearSecret
            ? { client_secret: '' }
            : secret.trim()
              ? { client_secret: secret.trim() }
              : {}),
        }
        // 확인 창 경고는 저장 전에 읽은 개수 기준(저장 응답은 새 issuer 기준).
        if (
          config &&
          !sameIssuer(body.issuer_url, config.issuer_url) &&
          res.passwordless > 0
        ) {
          save.reset()
          setPending(body)
          return
        }
        submit(body)
      }}
    >
      <Field
        label={t('oidc.buttonLabel')}
        name="button_label"
        defaultValue={config?.button_label ?? ''}
        hint={t('oidc.buttonLabelHint')}
        autoComplete="off"
      />
      <Field
        label={t('oidc.issuerUrl')}
        name="issuer_url"
        type="url"
        value={issuer}
        onChange={(e) => {
          setIssuer(e.target.value)
        }}
        required
        autoCapitalize="none"
        spellCheck={false}
      />
      <Field
        label={t('oidc.clientId')}
        name="client_id"
        defaultValue={config?.client_id ?? ''}
        required
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
      />
      <Field
        label={t('oidc.clientSecret')}
        name="client_secret"
        type="password"
        value={secret}
        onChange={(e) => {
          setSecret(e.target.value)
        }}
        autoComplete="new-password"
        disabled={clearSecret}
        placeholder={config?.secret_set ? t('oidc.secretSet') : undefined}
        hint={t(config?.secret_set ? 'oidc.secretHintSet' : 'oidc.secretHint')}
      />
      {config?.secret_set && (
        <Label className="flex items-center gap-2 font-normal">
          <input
            type="checkbox"
            checked={clearSecret}
            onChange={(e) => {
              setClearSecret(e.target.checked)
              if (e.target.checked) setSecret('')
            }}
          />
          {t('oidc.clearSecret')}
        </Label>
      )}
      <CallbackAddress url={res.callback_url} />
      <FormError error={pending ? null : save.error} />
      <div>
        <Button type="submit" disabled={save.isPending}>
          {t('common.save')}
        </Button>
      </div>
      <Dialog
        open={pending !== null}
        onOpenChange={(open) => {
          if (!open) setPending(null)
        }}
      >
        <DialogContent role="alertdialog">
          <DialogHeader>
            <DialogTitle>{t('oidc.changeIssuerTitle')}</DialogTitle>
            <DialogDescription>
              {t('oidc.warn', {
                linked: res.linked,
                passwordless: res.passwordless,
              })}
            </DialogDescription>
          </DialogHeader>
          <FormError error={save.error} />
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setPending(null)
              }}
            >
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              disabled={save.isPending}
              onClick={() => {
                if (pending) submit(pending)
              }}
            >
              {t('oidc.changeIssuerAction')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </form>
  )
}

function DeleteSection({ res }: { res: OidcResponse }) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const [confirming, setConfirming] = useState(false)
  const remove = useMutation({
    mutationFn: () => call(api.DELETE('/api/oidc')),
    onSuccess: () => {
      qc.setQueryData<OidcResponse>(oidcQueryKey, {
        config: null,
        callback_url: res.callback_url,
        linked: 0,
        passwordless: 0,
      })
      setConfirming(false)
      toast(t('oidc.deleted'))
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
          {t('oidc.delete')}
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
            <DialogTitle>{t('oidc.deleteTitle')}</DialogTitle>
            <DialogDescription>{t('oidc.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          {res.passwordless > 0 && (
            <p className="text-sm text-destructive">
              {t('oidc.warn', {
                linked: res.linked,
                passwordless: res.passwordless,
              })}
            </p>
          )}
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
              {t('oidc.deleteAction')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  )
}

export function OidcSection({ res }: { res: OidcResponse }) {
  return (
    <div className="grid gap-6">
      {/* 저장, 삭제 뒤 폼 상태 재초기화. */}
      <ConfigForm key={JSON.stringify(res)} res={res} />
      {res.config && <DeleteSection res={res} />}
    </div>
  )
}
