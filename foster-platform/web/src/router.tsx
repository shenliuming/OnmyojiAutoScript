import { Route, Routes, useParams } from 'react-router-dom'
import { AdminPage } from './pages/AdminPage'

type CustomerRouteParams = { token: string }

function CustomerPlaceholder({ kind }: { kind: 'login' | 'service' }) {
  const { token } = useParams<CustomerRouteParams>()
  if (!token) return <NotFound />
  return <main className="placeholder">
    <h1>{kind === 'login' ? '扫码登录' : '服务控制'}</h1>
    <p>{kind === 'login' ? '客户登录页面将在下一阶段接入。' : '客户服务页面将在下一阶段接入。'}</p>
  </main>
}

function NotFound() {
  return <main className="placeholder"><h1>页面不存在</h1><a href="/admin">返回控制台</a></main>
}

export function AppRouter() {
  return <Routes>
    <Route path="/admin" element={<AdminPage />} />
    <Route path="/login/:token" element={<CustomerPlaceholder kind="login" />} />
    <Route path="/service/:token" element={<CustomerPlaceholder kind="service" />} />
    <Route path="*" element={<NotFound />} />
  </Routes>
}
