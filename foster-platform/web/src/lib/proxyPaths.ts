// Vite treats proxy keys beginning with ^ as regular expressions.
export const BACKEND_PROXY_PATTERNS = [
  '^/admin/',
  '^/public/login/',
  '^/r/',
  '^/healthz(?:\\?|$)',
  '^/readyz(?:\\?|$)',
  '^/agent/ws(?:\\?|$)',
] as const

export function isBackendProxyPath(path: string): boolean {
  return BACKEND_PROXY_PATTERNS.some((pattern) => new RegExp(pattern).test(path))
}
