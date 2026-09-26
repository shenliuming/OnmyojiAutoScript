export function QrPanel({ publicToken, qrExpiresAt }: { publicToken: string; qrExpiresAt: string }) {
  const source = `/public/login/${encodeURIComponent(publicToken)}/qr?v=${encodeURIComponent(qrExpiresAt)}`
  return <div className="qr-panel"><img src={source} alt="登录二维码" width="300" height="300" /></div>
}
