import { readAdminToken } from './adminAuth'

export async function apiFetch(path: string, init: RequestInit = {}): Promise<Response> {
  if (!path.startsWith('/') || path.startsWith('//')) {
    throw new Error('必须使用本站相对 API 地址')
  }
  const headers = new Headers(init.headers)
  if (path.startsWith('/admin/')) {
    const token = readAdminToken()
    if (!token) throw new Error('管理凭据缺失，请从本机一键启动器打开控制台')
    headers.set('Authorization', `Bearer ${token}`)
  }
  return fetch(path, { ...init, headers, referrerPolicy: 'no-referrer' })
}
