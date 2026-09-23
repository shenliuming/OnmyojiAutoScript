# Foster Platform 首机部署

本目录用于把当前 MVP 跑成一套可实际验收的环境：

- Linux / 云服务器：MySQL + Foster Server
- Windows 宿主机：模拟器 + OAS + Foster Agent
- 浏览器：扫码登录 H5 / 服务状态 H5

## 1. 启动 Server

进入 `foster-platform/`：

```bash
cp prod.env.example prod.env
```

修改 `prod.env`，至少替换：

- `MYSQL_PASSWORD`
- `MYSQL_ROOT_PASSWORD`
- `FOSTER_AGENT_TOKEN`
- `FOSTER_ADMIN_TOKEN`

生产环境不要提交 `prod.env`。

启动：

```bash
docker compose --env-file prod.env -f docker-compose.prod.yml up -d --build
```

查看：

```bash
docker compose --env-file prod.env -f docker-compose.prod.yml ps
docker compose --env-file prod.env -f docker-compose.prod.yml logs -f server
```

探活：

```bash
curl -fsS http://127.0.0.1:8080/healthz
curl -fsS http://127.0.0.1:8080/readyz
```

预期：

```json
{"status":"ready"}
```

Server 启动时会自动执行 SQLx migrations。

## 2. 创建第一台 Windows Host

Agent 不允许自己创建未知 Host。先通过管理 API bootstrap：

```bash
curl -X POST http://SERVER:8080/admin/hosts \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"hostCode":"HOST-01","hostname":"win-host-01"}'
```

返回示例：

```json
{
  "id": 1,
  "hostCode": "HOST-01",
  "hostname": "win-host-01",
  "status": "OFFLINE",
  "agentVersion": null,
  "lastHeartbeatAt": null,
  "totalEmulators": 0,
  "onlineEmulators": 0,
  "configuredCapacity": 0,
  "boundAccounts": 0
}
```

记住 `id`，它就是 Windows Agent 的 `FOSTER_HOST_ID`。

查看所有 Host：

```bash
curl http://SERVER:8080/admin/hosts \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN"
```

## 3. 准备 Windows OAS

在 OAS 根目录，用现有 Python 环境启动 FastAPI：

```powershell
python server.py --host 127.0.0.1 --port 22270
```

Agent 只访问本机 OAS，不需要把 22270 暴露到公网。

每个模拟器需要有一个对应 OAS config，例如：

```text
emu-01 -> oas-emu-01
emu-02 -> oas-emu-02
```

`FOSTER_EMULATORS_JSON` 里的 `oasConfigName` 必须与实际 OAS config 名一致。

## 4. 编译 / 放置 Windows Agent

在 Windows Rust 环境：

```powershell
cd foster-platform
cargo build --release -p foster-agent
```

准备目录，例如：

```text
C:\FosterAgent\
├── foster-agent.exe
├── Start-FosterAgent.ps1
├── Install-FosterAgentTask.ps1
└── agent.env
```

从：

```text
deploy/windows/agent.env.example
```

复制为 `agent.env`。

至少修改：

- `FOSTER_SERVER_WS_URL`
- `FOSTER_AGENT_TOKEN`，必须和 Server 一致
- `FOSTER_HOST_ID`，填写上一步返回的 Host id
- `FOSTER_ADB_PATH`
- `FOSTER_EMULATORS_JSON`

示例：

```dotenv
FOSTER_SERVER_WS_URL=ws://10.0.0.10:8080/agent/ws
FOSTER_AGENT_TOKEN=your-shared-agent-token
FOSTER_AGENT_ID=host-01-agent
FOSTER_HOST_ID=1
FOSTER_OAS_BASE_URL=http://127.0.0.1:22270
FOSTER_ADB_PATH=C:\Android\platform-tools\adb.exe
FOSTER_EMULATORS_JSON=[{"emulatorCode":"emu-01","adbSerial":"127.0.0.1:16384","oasConfigName":"oas-emu-01","packageName":"com.netease.onmyoji","startProgram":null,"startArgs":[],"stopProgram":null,"stopArgs":[],"loginPrepareProgram":null,"loginPrepareArgs":[],"loginPrepareDelayMs":3000}]
```

先前台启动：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\Start-FosterAgent.ps1
```

确认稳定后再安装开机自启：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\Install-FosterAgentTask.ps1 -AgentDir C:\FosterAgent
Start-ScheduledTask -TaskName FosterAgent
```

## 5. 验证 Host / Emulator 在线

再次查询：

```bash
curl http://SERVER:8080/admin/hosts \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN"
```

应看到：

- Host status = `ONLINE`
- `agentVersion` 有值
- `lastHeartbeatAt` 持续刷新
- `totalEmulators > 0`
- ADB 正常的模拟器计入 `onlineEmulators`

如果 Host 一直 OFFLINE，优先检查：

1. Agent token 是否和 Server 一致；
2. `FOSTER_HOST_ID` 是否为 bootstrap 返回的 id；
3. Windows 到 Server 的 8080 / WebSocket 是否可达；
4. Agent 日志；
5. ADB serial 是否正确。

## 6. 创建首个 BASIC 用户

调用 onboarding：

```bash
curl -X POST http://SERVER:8080/admin/onboard \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "customerId": 10001,
    "planCode": "BASIC_AUTO_FOSTER",
    "serviceDays": 30,
    "loginTtlMinutes": 15
  }'
```

返回内容包含登录 URL 和服务 URL。

验收：

1. 打开登录 H5；
2. 扫描游戏二维码；
3. 系统检测账号 / 角色 / 区服；
4. 确认登录；
5. Subscription 进入 ACTIVE；
6. 打开 Service H5；
7. 查看今日执行次数、下次执行时间、暂停/不上号时段。

## 7. PLATFORM 资源准备

先创建 provider：

```bash
curl -X POST http://SERVER:8080/admin/providers \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "providerCode": "P-001",
    "nickname": "资源号1",
    "providerAlias": "资源A01",
    "serverName": "春之樱"
  }'
```

然后：

1. 给客户账号建立 VERIFIED friend binding；
2. 创建 `FISH` 或 `TAIKO_JADE` resource cycle；
3. 确认 cycle 为 AVAILABLE；
4. 再创建 PLATFORM 用户。

资源池查看：

```bash
curl http://SERVER:8080/admin/resource-pool \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN"
```

## 8. 首次端到端 Smoke Checklist

部署完成后逐项确认：

- [ ] `/healthz` = 200
- [ ] `/readyz` = 200
- [ ] Host bootstrap 成功
- [ ] Agent Host = ONLINE
- [ ] Emulator inventory 已上报
- [ ] OAS 22270 只监听本机
- [ ] BASIC onboarding 成功
- [ ] QR 页面能显示模拟器二维码
- [ ] 扫码后识别到真实 OCR 身份
- [ ] Subscription 激活
- [ ] Scheduler 创建 FosterJob
- [ ] Agent 收到 ExecuteFoster
- [ ] OAS 完成一次单次寄养
- [ ] Job 进入 SUCCESS 或结构化 RETRY
- [ ] SUCCESS 后 next_run_at 基于实际完成时间
- [ ] Service H5 能查看状态
- [ ] “我要玩游戏”暂停有效
- [ ] 不上号时段顺延有效

## 9. 安全边界

- 不保存游戏密码。
- Agent token 和 Admin token 必须使用不同随机值。
- 不要公开 OAS 的 22270 端口。
- MySQL 在 production compose 中没有映射宿主机端口。
- Admin API 应放在可信网络、VPN 或反向代理鉴权之后。
- 对公网提供 H5 / Agent WebSocket 时建议使用 HTTPS/WSS。
- `prod.env`、`agent.env` 不提交 Git。
