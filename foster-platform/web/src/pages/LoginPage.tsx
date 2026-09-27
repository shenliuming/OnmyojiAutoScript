import { useCallback, useEffect, useState } from 'react'
import { useLocation, useParams } from 'react-router-dom'
import { IdentityForm, type Identity } from '../components/IdentityForm'
import { QrPanel } from '../components/QrPanel'
import { apiFetch } from '../lib/api'

type LoginStatus = {
  sessionNo: string
  status: string
  qrExpiresAt: string | null
  characterName: string | null
  serverName: string | null
  identityVerified: boolean
  identityVerifyReason: string | null
  expiresAt: string
}

const STATUS_LABELS: Record<string, string> = {
  CREATED: '请填写登录信息',
  WAITING_EMULATOR: '等待模拟器',
  PREPARING: '正在准备模拟器',
  QR_READY: '请扫码登录',
  WAITING_SCAN: '请扫码登录',
  DETECTING_LOGIN: '正在选择平台和角色',
  VERIFYING_ACCOUNT: '请确认账号',
  SUCCESS: '登录完成',
  FAILED: '登录失败',
  CANCELLED: '登录已取消',
}

export function LoginPage() {
  const { token } = useParams<{ token: string }>()
  const { hash } = useLocation()
  const controlToken = new URLSearchParams(hash.slice(1)).get('control')
  const [data, setData] = useState<LoginStatus | null>(null)
  const [statusMessage, setStatusMessage] = useState('正在连接…')
  const [detail, setDetail] = useState('')
  const [startBusy, setStartBusy] = useState(false)
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
      setStatusMessage(STATUS_LABELS[next.status] || next.status)

      const who = [next.characterName, next.serverName].filter(Boolean).join(' · ')
      if (next.status === 'SUCCESS') {
        setDetail(who || '账号已绑定，可以关闭此页面')
      } else if (next.status === 'VERIFYING_ACCOUNT' && next.identityVerified) {
        setDetail(who ? `已识别：${who}。请确认这是你的角色。` : '账号已识别，请确认')
      } else {
        setDetail(next.identityVerifyReason || who || '')
      }
    } catch {
      setStatusMessage('网络连接异常')
    }
  }, [token])

  useEffect(() => {
    void refresh()
    const timer = window.setInterval(() => { void refresh() }, 2000)
    let events: EventSource | undefined
    if (token && typeof EventSource !== 'undefined') {
      events = new EventSource(`/public/login/${encodeURIComponent(token)}/events`)
      events.addEventListener('login_status', () => { void refresh() })
    }
    return () => {
      window.clearInterval(timer)
      events?.close()
    }
  }, [refresh, token])

  async function startLogin(identity: Identity) {
    if (!controlToken || startBusy) return
    setStartBusy(true)
    setDetail('正在分配并启动模拟器…')
    try {
      const response = await apiFetch(`/public/login/${encodeURIComponent(controlToken)}/start`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(identity),
      })
      if (!response.ok) {
        setDetail(response.status === 400
          ? '请检查平台、角色名和角色 ID'
          : '启动登录失败，请检查模拟器状态后重试')
        return
      }
      await refresh()
    } catch {
      setDetail('网络异常，启动登录失败，请重试')
    } finally {
      setStartBusy(false)
    }
  }

  async function confirm() {
    if (!controlToken || !data?.identityVerified || data.status !== 'VERIFYING_ACCOUNT' || confirmBusy) return
    setConfirmBusy(true)
    try {
      const response = await apiFetch(`/public/login/${encodeURIComponent(controlToken)}/confirm`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ confirmed: true }),
      })
      if (!response.ok) setDetail('确认失败，请稍后重试')
      else await refresh()
    } catch {
      setDetail('网络异常，确认失败，请重试')
    } finally {
      setConfirmBusy(false)
    }
  }

  const showStartForm = Boolean(controlToken && data && ['CREATED', 'WAITING_EMULATOR'].includes(data.status))
  const qrVisible = Boolean(data && ['QR_READY', 'WAITING_SCAN'].includes(data.status) && data.qrExpiresAt)
  const canConfirm = Boolean(controlToken && data?.status === 'VERIFYING_ACCOUNT' && data.identityVerified && !confirmBusy)

  return <main className="customer-page"><section className="card">
    <h1>游戏账号登录</h1>
    <p className="muted">填写平台、角色名和角色 ID 后扫码。系统会自动选择平台和角色，并识别实际区服。</p>

    <div className="login-status" role="status">
      <strong>{statusMessage}</strong>
      <p>{detail}</p>
    </div>

    {showStartForm && <IdentityForm busy={startBusy} onSubmit={identity => { void startLogin(identity) }} />}

    {qrVisible && <>
      <QrPanel publicToken={token!} qrExpiresAt={data!.qrExpiresAt!} />
      <p className="muted">扫码后无需继续操作，系统会自动选择你填写的平台和角色。</p>
    </>}

    {data?.status === 'DETECTING_LOGIN' && <p className="muted">正在模拟器中选择平台、查找角色并读取区服…</p>}

    <div className="actions">
      <button onClick={() => { void refresh() }}>刷新状态</button>
      <button disabled={!canConfirm} onClick={() => { void confirm() }}>确认这是我的角色</button>
    </div>
  </section></main>
}
