import { useCallback, useEffect, useState, type FormEvent } from 'react'
import { apiFetch } from '../lib/api'
import { readAdminToken } from '../lib/adminAuth'

type Host = { id: number; hostname: string; status: string; totalEmulators: number; onlineEmulators: number; configuredCapacity: number }
type Emulator = { id: number; emulatorCode: string; adbSerial: string | null; status: string; maxAccountCount: number; boundAccounts: number }
type OnboardResult = { loginUrl: string; serviceUrl: string; loginDispatchStatus: string }

const CUSTOMER_ORIGIN = 'http://j55d6643.natappfree.cc'

async function readJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    let detail = `HTTP ${response.status}`
    try {
      const body = await response.json() as { message?: string; error?: string }
      detail = body.message || body.error || detail
    } catch { /* Non-JSON errors retain their HTTP status. */ }
    throw new Error(detail)
  }
  return response.json() as Promise<T>
}

export function AdminPage() {
  const [hasToken] = useState(() => Boolean(readAdminToken()))
  const [backendState, setBackendState] = useState('检查中…')
  const [agentState, setAgentState] = useState('检查中…')
  const [host, setHost] = useState<Host | null>(null)
  const [emulators, setEmulators] = useState<Emulator[]>([])
  const [deviceMessage, setDeviceMessage] = useState('正在获取设备…')
  const [message, setMessage] = useState(hasToken ? '' : '管理凭据缺失，请从本机一键启动器打开控制台')
  const [creating, setCreating] = useState(false)
  const [result, setResult] = useState<{ login: string; service: string } | null>(null)
  const [customerId, setCustomerId] = useState(10005)
  const [serviceDays, setServiceDays] = useState(30)
  const [loginTtlMinutes, setLoginTtlMinutes] = useState(30)
  const [planCode, setPlanCode] = useState('BASIC_AUTO_FOSTER')

  const refresh = useCallback(async () => {
    try {
      const response = await apiFetch('/readyz', { cache: 'no-store' })
      setBackendState(response.ok ? '在线' : '离线')
    } catch { setBackendState('无法连接') }

    if (!hasToken) {
      setAgentState('管理凭据缺失')
      setDeviceMessage('没有管理凭据')
      return
    }
    try {
      const hosts = await readJson<Host[]>(await apiFetch('/admin/hosts'))
      const current = hosts[0]
      setHost(current || null)
      if (!current) {
        setAgentState('未登记主机')
        setEmulators([])
        setDeviceMessage('没有登记的模拟器主机')
        return
      }
      setAgentState(current.status === 'ONLINE' ? '在线' : '离线')
      const devices = await readJson<Emulator[]>(await apiFetch(`/admin/hosts/${current.id}/emulators`))
      setEmulators(devices)
      setDeviceMessage(devices.length ? '' : '暂未发现设备')
    } catch (error) {
      setAgentState('状态读取失败')
      setHost(null)
      setEmulators([])
      setDeviceMessage(`读取失败：${error instanceof Error ? error.message : '未知错误'}`)
    }
  }, [hasToken])

  useEffect(() => {
    void refresh()
    const timer = window.setInterval(() => { void refresh() }, 5000)
    return () => window.clearInterval(timer)
  }, [refresh])

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!hasToken) return
    setCreating(true)
    setMessage('正在分配空闲模拟器并创建登录会话…')
    setResult(null)
    try {
      const response = await apiFetch('/admin/onboard', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ customerId, planCode: planCode.trim(), serviceDays, loginTtlMinutes }),
      })
      const data = await readJson<OnboardResult>(response)
      const login = new URL(data.loginUrl, CUSTOMER_ORIGIN).href
      const service = new URL(data.serviceUrl, CUSTOMER_ORIGIN).href
      setResult({ login, service })
      setMessage(`登录会话已创建（${data.loginDispatchStatus}）`)
      await refresh()
    } catch (error) {
      setMessage(`创建失败：${error instanceof Error ? error.message : '未知错误'}`)
    } finally { setCreating(false) }
  }

  async function copyLogin() {
    if (!result) return
    try {
      await navigator.clipboard.writeText(result.login)
      setMessage('客户扫码链接已复制')
    } catch { setMessage('复制失败，请手动复制上方链接') }
  }

  return <main className="admin-page">
    <header className="page-header">
      <div><h1>阴阳师寄养控制台</h1><p className="muted">本机管理页面 · 状态每 5 秒自动刷新</p></div>
      <button type="button" onClick={() => { void refresh() }}>立即刷新</button>
    </header>
    <div className="grid">
      <section className="card"><h2>服务状态</h2>
        <div className="state"><i className={`dot ${backendState === '在线' ? 'online' : 'offline'}`} /><span>后端</span><strong>{backendState}</strong></div>
        <div className="state"><i className={`dot ${agentState === '在线' ? 'online' : 'offline'}`} /><span>Agent</span><strong>{agentState}</strong></div>
        {host && <p className="small">{host.hostname} · {host.onlineEmulators}/{host.totalEmulators} 台在线 · 可用容量 {host.configuredCapacity}</p>}
      </section>
      <section className="card"><h2>扫码登录</h2>
        <form onSubmit={event => { void create(event) }}>
          <div className="formgrid">
            <label>客户编号<input type="number" min="1" required value={customerId} onChange={event => setCustomerId(Number(event.target.value))} /></label>
            <label>服务天数<input type="number" min="1" required value={serviceDays} onChange={event => setServiceDays(Number(event.target.value))} /></label>
            <label>扫码链接有效分钟<input type="number" min="1" max="1440" required value={loginTtlMinutes} onChange={event => setLoginTtlMinutes(Number(event.target.value))} /></label>
            <label>套餐代码<input required value={planCode} onChange={event => setPlanCode(event.target.value)} /></label>
          </div>
          <button type="submit" disabled={!hasToken || creating}>分配模拟器并创建扫码登录</button>
        </form>
        <p role="status" className="muted message">{message}</p>
        {result && <div className="result">
          <p><a href={result.login} target="_blank" rel="noreferrer">打开客户扫码页面（外网）</a></p>
          <p className="small url">{result.login}</p>
          <p><a href={result.service} target="_blank" rel="noreferrer">打开服务控制页面（外网）</a></p>
          <button type="button" onClick={() => { void copyLogin() }}>复制客户扫码链接</button>
        </div>}
        <p className="small">Agent 和空闲模拟器在线后才能创建登录会话。</p>
      </section>
      <section className="card wide"><h2>模拟器</h2>
        {emulators.length ? <div className="table-wrap"><table><thead><tr><th>设备</th><th>ADB 地址</th><th>状态</th><th>账号容量</th><th>已绑定</th></tr></thead>
          <tbody>{emulators.map(device => <tr key={device.id}><td>{device.emulatorCode}</td><td>{device.adbSerial || '—'}</td><td>{device.status}</td><td>{device.maxAccountCount}</td><td>{device.boundAccounts}</td></tr>)}</tbody></table></div> : <p className="muted">{deviceMessage}</p>}
      </section>
      <section className="card wide"><h2>使用说明</h2><p className="muted">确认后端、Agent 和模拟器都在线，再创建扫码登录。二维码页面可复制链接发给客户。Agent 的运行日志在另一个控制台窗口。</p></section>
    </div>
  </main>
}
