const STORAGE_KEY = 'foster.adminToken'

/** Consume a launcher's fragment credential before any application request. */
export function readAdminToken(): string | null {
  const fragment = window.location.hash.slice(1)
  const params = new URLSearchParams(fragment)
  if (params.has('token')) {
    const token = params.get('token') || null
    params.delete('token')
    const remaining = params.toString()
    window.history.replaceState(window.history.state, '',
      window.location.pathname + window.location.search + (remaining ? `#${remaining}` : ''))
    if (token) sessionStorage.setItem(STORAGE_KEY, token)
    return token || sessionStorage.getItem(STORAGE_KEY)
  }
  return sessionStorage.getItem(STORAGE_KEY)
}
