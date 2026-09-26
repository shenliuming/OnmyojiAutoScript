import { useCallback, useEffect, useState } from 'react'
import { useLocation, useParams } from 'react-router-dom'
import { IdentityForm, type Identity } from '../components/IdentityForm'
import { QrPanel } from '../components/QrPanel'
import { apiFetch } from '../lib/api'

type LoginStatus = {
  sessionNo: string; status: string; qrExpiresAt: string | null;
  characterName: string | null; serverName: string | null; identityVerified: boolean;
  identityVerifyReason: string | null; expiresAt: string;
}

export function LoginPage() {
  const { token } = useParams<{ token: string }>()
  const { hash } = useLocation()
  const controlToken = new URLSearchParams(hash.slice(1)).get('control')
  const [data, setData] = useState<LoginStatus | null>(null)
  const [statusMessage, setStatusMessage] = useState('正在连接…')
  const [detail, setDetail] = useState('')
  const [platformBusy, setPlatformBusy] = useState(false)
  const [identityBusy, setIdentityBusy] = useState(false)
  const [confirmBusy, setConfirmBusy] = useState(false)

  const refresh = useCallback(async () => {
    if (!token) return
    try {
      const response = await apiFetch(`/public/login/${encodeURIComponent(token)}/meta`, { cache: 'no-store' })
      if (!response.ok) {
        setData(null)
        setStatusMessage(response.status === 410 ? '登录链接已过期' : '无法读取登录状态')
        return
      }
      const next = await response.json() as LoginStatus
      setData(next)
      setStatusMessage(next.status === 'SUCCESS' ? '登录完成' : next.status)
      const who = [next.characterName, next.serverName].filter(Boolean).join(' · ')
      setDetail(next.status === 'SUCCESS' ? who || '账号已绑定，可以关闭此页面' : next.identityVerifyReason || who || '等待游戏返回账号信息')
    } catch { setStatusMessage('网络连接异常') }
  }, [token])

  useEffect(() => {
    void refresh()
    const timer = window.setInterval(() => { void refresh() }, 2000)
    let events: EventSource | undefined
    if (token && typeof EventSource !== 'undefined') {
      events = new EventSource(`/public/login/${encodeURIComponent(token)}/events`)
      events.addEventListener('login_status', () => { void refresh() })
      // EventSource reconnects automatically; polling also covers disconnected clients.
    }
    return () => { window.clearInterval(timer); events?.close() }
  }, [refresh, token])

  async function selectPlatform(platform: 'android' | 'ios') {
    if (!controlToken || platformBusy) return
    setPlatformBusy(true)
    setDetail('已提交平台选择，正在继续登录…')
    try {
      const response = await apiFetch(`/public/login/${encodeURIComponent(controlToken)}/platform`, {
        method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ platform }),
      })
      if (!response.ok) setDetail('平台选择失败，请重新点击')
      else await refresh()
    } catch { setDetail('网络异常，平台选择失败，请重试') }
    finally { setPlatformBusy(false) }
  }

  async function submitIdentity(identity: Identity) {
    if (!controlToken || identityBusy) return
    setIdentityBusy(true)
    setDetail('已提交账号信息，等待 Agent 截图/OCR 校验…')
    try {
      const response = await apiFetch(`/public/login/${encodeURIComponent(controlToken)}/identity`, {
        method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(identity),
      })
      if (!response.ok) {
        setDetail(response.status === 409 ? '当前登录状态不能提交，请等扫码后再试' : '提交失败，请检查填写内容后重试')
        return
      }
      const result = await response.json() as { verified: boolean; message: string }
      setDetail(result.message)
      if (result.verified) await refresh()
    } catch { setDetail('网络异常，提交失败，请重试') }
    finally { setIdentityBusy(false) }
  }

  async function confirm() {
    if (!controlToken || !data?.identityVerified || data.status !== 'VERIFYING_ACCOUNT' || confirmBusy) return
    setConfirmBusy(true)
    try {
      const response = await apiFetch(`/public/login/${encodeURIComponent(controlToken)}/confirm`, {
        method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ confirmed: true }),
      })
      if (!response.ok) setDetail('确认失败，请稍后重试')
      else await refresh()
    } catch { setDetail('网络异常，确认失败，请重试') }
    finally { setConfirmBusy(false) }
  }

  const qrVisible = data && ['QR_READY', 'WAITING_SCAN'].includes(data.status) && Boolean(data.qrExpiresAt)
  const choosePlatform = controlToken && data && ['QR_READY', 'WAITING_SCAN'].includes(data.status)
  const enterIdentity = controlToken && data && ['DETECTING_LOGIN', 'VERIFYING_ACCOUNT'].includes(data.status) && !data.identityVerified
  const canConfirm = Boolean(controlToken && data?.status === 'VERIFYING_ACCOUNT' && data.identityVerified && !confirmBusy)

  return <main className="customer-page"><section className="card">
    <h1>游戏账号登录</h1>
    <p className="muted">请使用手机扫码完成登录。系统只保存账号识别信息，不保存密码。</p>
    <div className="login-status" role="status"><strong>{statusMessage}</strong><p>{detail}</p></div>
    {qrVisible && <QrPanel publicToken={token!} qrExpiresAt={data.qrExpiresAt!} />}
    {choosePlatform && <div className="platform-box"><p>扫码成功后，请选择账号所在的区服平台：</p>
      <div className="actions"><button disabled={platformBusy} onClick={() => { void selectPlatform('android') }}>安卓区</button>
        <button disabled={platformBusy} onClick={() => { void selectPlatform('ios') }}>iOS 区</button></div>
    </div>}
    {enterIdentity && <IdentityForm busy={identityBusy} onSubmit={identity => { void submitIdentity(identity) }} />}
    <div className="actions"><button onClick={() => { void refresh() }}>刷新状态</button>
      <button disabled={!canConfirm} onClick={() => { void confirm() }}>确认这是我的账号</button></div>
  </section></main>
}
