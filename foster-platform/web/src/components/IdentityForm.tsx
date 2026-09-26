import { useState, type FormEvent } from 'react'

export type Identity = { serverName: string; characterName: string; gameUid: string }

export function IdentityForm({ busy, onSubmit }: { busy: boolean; onSubmit: (identity: Identity) => void }) {
  const [serverName, setServerName] = useState('')
  const [characterName, setCharacterName] = useState('')
  const [gameUid, setGameUid] = useState('')
  const [error, setError] = useState('')

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const identity = { serverName: serverName.trim(), characterName: characterName.trim(), gameUid: gameUid.trim() }
    if (!identity.serverName || !identity.characterName || !identity.gameUid) {
      setError('请填写区服、角色名和游戏 UID')
      return
    }
    setError('')
    onSubmit(identity)
  }

  return <form className="identity-form" onSubmit={submit} noValidate>
    <h2>填写账号识别信息</h2>
    <p className="muted">游戏内弹窗、公告和“跳过”按钮由 Agent 在模拟器中处理，这里只填写账号信息。</p>
    <label htmlFor="identity-server">区服（可搜索）</label>
    <input id="identity-server" list="serverOptions" autoComplete="off" placeholder="选择或输入区服" value={serverName} onChange={event => setServerName(event.target.value)} />
    <datalist id="serverOptions" />
    <label htmlFor="identity-character">角色名</label>
    <input id="identity-character" autoComplete="off" placeholder="填写游戏内角色名" value={characterName} onChange={event => setCharacterName(event.target.value)} />
    <label htmlFor="identity-uid">游戏 UID</label>
    <input id="identity-uid" inputMode="numeric" autoComplete="off" placeholder="填写游戏 UID" value={gameUid} onChange={event => setGameUid(event.target.value)} />
    {error && <p role="alert">{error}</p>}
    <button type="submit" disabled={busy}>提交，开始校验</button>
  </form>
}
