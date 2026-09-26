import { useCallback, useEffect, useState } from 'react'
import { useLocation, useParams } from 'react-router-dom'
import { apiFetch } from '../lib/api'

type QuietPeriod = { weekdayMask: number; startTime: string; endTime: string; beforeBufferMinutes: number; afterBufferMinutes: number }
type Job = { status: string; scheduledAt: string; finishedAt: string | null; resultMessage: string | null }
type ServiceStatus = {
  subscriptionNo: string; serviceStatus: string; characterName: string | null; serverName: string | null;
  todaySuccessCount: number; dailyTargetRuns: number; planName: string; resourceType: string | null;
  nextRunAt: string | null; effectiveBlockedUntil: string | null; blockReason: string | null;
  reloginRequired: boolean; quietPeriods: QuietPeriod[]; recentJobs: Job[];
}

function fmt(value: string | null | undefined) { return value ? new Date(value).toLocaleString() : '—' }

export function ServicePage() {
  const { token } = useParams<{ token: string }>()
  const { hash } = useLocation()
  const controlToken = new URLSearchParams(hash.slice(1)).get('control')
  const [status, setStatus] = useState<ServiceStatus | null>(null)
  const [quiet, setQuiet] = useState('[]')
  const [quietEditing, setQuietEditing] = useState(false)
  const [message, setMessage] = useState('加载中…')
  const [busy, setBusy] = useState(false)

  const refresh = useCallback(async () => {
    if (!token) return
    try {
      const response = await apiFetch(`/r/${encodeURIComponent(token)}`, { cache: 'no-store' })
      if (!response.ok) {
        setStatus(null)
        setMessage(response.status === 410 ? '服务链接已过期' : response.status === 404 ? '服务链接无效' : '无法读取服务状态')
        return
      }
      const next = await response.json() as ServiceStatus
      setStatus(next)
      setMessage('')
      if (!quietEditing) setQuiet(JSON.stringify(next.quietPeriods || [], null, 2))
    } catch { setMessage('网络连接异常') }
  }, [token, quietEditing])

  useEffect(() => {
    void refresh()
    const timer = window.setInterval(() => { void refresh() }, 5000)
    return () => window.clearInterval(timer)
  }, [refresh])

  async function control(path: string, options: RequestInit) {
    if (!controlToken || busy) return
    setBusy(true)
    try {
      const response = await apiFetch(`/r/${encodeURIComponent(controlToken)}${path}`, options)
      if (!response.ok) throw new Error(response.status === 410 ? '控制链接已过期' : `HTTP ${response.status}`)
      setMessage('设置已保存')
      await refresh()
    } catch (error) { setMessage(`设置失败：${error instanceof Error ? error.message : '网络异常'}`) }
    finally { setBusy(false) }
  }

  function saveQuiet() {
    let parsed: unknown
    try { parsed = JSON.parse(quiet || '[]') } catch { setMessage('时间段格式不正确'); return }
    if (!Array.isArray(parsed) || parsed.some(item => !item || typeof item !== 'object' ||
      !Number.isInteger(item.weekdayMask) || typeof item.startTime !== 'string' || typeof item.endTime !== 'string')) {
      setMessage('时间段格式不正确'); return
    }
    const quietPeriods: QuietPeriod[] = parsed.map(item => ({
      weekdayMask: item.weekdayMask, startTime: item.startTime, endTime: item.endTime,
      beforeBufferMinutes: item.beforeBufferMinutes || 0, afterBufferMinutes: item.afterBufferMinutes || 0,
    }))
    void control('/quiet-periods', { method: 'PUT', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ quietPeriods }) })
  }

  return <main className="service-page">
    <header className="page-header"><div><h1>我的寄养服务</h1>
      <p className="muted">{status ? [status.characterName, status.serverName].filter(Boolean).join(' · ') || status.subscriptionNo : message}</p>
    </div>{!controlToken && <span className="muted">只读链接</span>}</header>
    {status && <div className="grid">
      <section className="card"><h2>服务状态</h2><p className="value">{status.reloginRequired ? '需要重新登录' : status.serviceStatus}</p></section>
      <section className="card"><h2>今日寄养</h2><p className="value">{status.todaySuccessCount} / {status.dailyTargetRuns}</p></section>
      <section className="card"><h2>套餐</h2><p className="value">{status.planName}{status.resourceType ? ` · ${status.resourceType}` : ''}</p></section>
      <section className="card"><h2>下次预计</h2><p className="value">{fmt(status.nextRunAt)}</p></section>
      <section className="card wide"><h2>临时暂停</h2><p>{status.effectiveBlockedUntil ? `${status.blockReason} 至 ${fmt(status.effectiveBlockedUntil)}` : '当前未暂停'}</p>
        <div className="actions">{(['1H', '2H', '4H', 'TODAY'] as const).map((preset, index) =>
          <button key={preset} disabled={!controlToken || busy} onClick={() => { void control('/pause', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ preset }) }) }}>
            {['暂停 1 小时', '暂停 2 小时', '暂停 4 小时', '今天不上号'][index]}
          </button>)}
          <button disabled={!controlToken || busy} onClick={() => { void control('/pause', { method: 'DELETE' }) }}>恢复运行</button></div>
      </section>
      <section className="card wide"><h2>不上号时间段</h2><p className="muted">JSON 数组，weekdayMask 1–127；支持跨午夜。</p>
        <label htmlFor="quiet-periods">不上号时间段</label>
        <textarea id="quiet-periods" value={quiet} disabled={!controlToken || busy} onFocus={() => setQuietEditing(true)} onBlur={() => setQuietEditing(false)} onChange={event => setQuiet(event.target.value)} />
        <div className="actions"><button disabled={!controlToken || busy} onClick={saveQuiet}>保存时间段</button></div>
      </section>
      <section className="card wide"><h2>最近执行</h2><div className="table-wrap"><table><thead><tr><th>状态</th><th>计划时间</th><th>完成时间</th><th>结果</th></tr></thead>
        <tbody>{status.recentJobs.map((job, index) => <tr key={index}><td>{job.status}</td><td>{fmt(job.scheduledAt)}</td><td>{fmt(job.finishedAt)}</td><td>{job.resultMessage || ''}</td></tr>)}</tbody></table></div>
      </section>
    </div>}
    <p role="status" className="muted">{message}</p>
  </main>
}
