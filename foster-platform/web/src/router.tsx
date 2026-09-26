import { Route, Routes } from 'react-router-dom'
import { AdminPage } from './pages/AdminPage'
import { LoginPage } from './pages/LoginPage'
import { ServicePage } from './pages/ServicePage'

function NotFound() {
  return <main className="placeholder"><h1>页面不存在</h1><a href="/admin">返回控制台</a></main>
}

export function AppRouter() {
  return <Routes>
    <Route path="/admin" element={<AdminPage />} />
    <Route path="/login/:token" element={<LoginPage />} />
    <Route path="/service/:token" element={<ServicePage />} />
    <Route path="*" element={<NotFound />} />
  </Routes>
}
