import { useState, type FormEvent } from 'react'

export type Identity = {
  platform: 'android' | 'ios'
  characterName: string
  gameUid: string
}

export function IdentityForm({ busy, onSubmit }: { busy: boolean; onSubmit: (identity: Identity) => void }) {
  const [platform, setPlatform] = useState<'android' | 'ios'>('android')
  const [characterName, setCharacterName] = useState('')
  const [gameUid, setGameUid] = useState('')
  const [error, setError] = useState('')

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const identity = {
      platform,
      characterName: characterName.trim(),
      gameUid: gameUid.trim(),
    }
    if (!identity.characterName || !identity.gameUid) {
      setError('请选择平台，并填写角色名和角色 ID')
      return
    }
    setError('')
    onSubmit(identity)
  }

  return <form className="identity-form" onSubmit={submit} noValidate>
    <h2>开始登录</h2>
    <p className="muted">只需要选择平台、填写角色名和角色 ID。区服由系统在模拟器里识别。</p>

    <label htmlFor="identity-platform">游戏平台</label>
    <select
      id="identity-platform"
      value={platform}
      onChange={event => setPlatform(event.target.value as 'android' | 'ios')}
      disabled={busy}
    >
      <option value="android">Android</option>
      <option value="ios">iOS</option>
    </select>

    <label htmlFor="identity-character">角色名</label>
    <input
      id="identity-character"
      autoComplete="off"
      placeholder="填写游戏内角色名"
      value={characterName}
      onChange={event => setCharacterName(event.target.value)}
      disabled={busy}
    />

    <label htmlFor="identity-uid">角色 ID</label>
    <input
      id="identity-uid"
      inputMode="numeric"
      autoComplete="off"
      placeholder="填写角色 ID"
      value={gameUid}
      onChange={event => setGameUid(event.target.value)}
      disabled={busy}
    />

    {error && <p role="alert">{error}</p>}
    <button type="submit" disabled={busy}>开始登录</button>
  </form>
}
