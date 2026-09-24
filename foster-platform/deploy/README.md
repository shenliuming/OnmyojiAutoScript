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

## 4. 下载 / 放置 Windows Agent

首机验证后，推荐使用 GitHub Actions 构建好的 Windows 包，不要求每台宿主机安装 Rust。

在 GitHub 仓库：

1. 打开 **Actions**；
2. 选择 **Build Foster Agent Windows Package**；
3. 点击 **Run workflow**；
4. 可选填写 `package_label`，例如 `host-01` 或 `2026-09-23`；
5. 等 workflow 完成后下载同名 artifact；
6. 解压其中的 zip，并校验旁边的 `.zip.sha256`。

包内包含：

```text
foster-agent.exe
Start-FosterAgent.ps1
Install-FosterAgentTask.ps1
Test-FosterHost.ps1
agent.env.example
foster-agent.exe.sha256
build-metadata.json
```

Windows 包仍可通过独立的手动 `workflow_dispatch` 构建。阶段末更新 `.github/ci-trigger/foster-platform` 时，完整 CI 也会调用同一套 Windows 构建与打包流程，并检查 zip、EXE SHA-256、文件清单及脚本语法；平时的代码 push 不运行它。CI 的 Windows artifact 可用于首机预部署验证，但真正的模拟器/OAS 联调仍须在 Windows 宿主机完成。包中不包含 Agent token、Admin token 或其他密钥，也不会自动创建 GitHub Release。

把内容解压到例如：

```text
C:\FosterAgent\
```

复制配置：

```powershell
Copy-Item .\agent.env.example .\agent.env
```

至少修改：

- `FOSTER_SERVER_WS_URL`
- `FOSTER_AGENT_TOKEN`，必须和 Server 一致
- `FOSTER_HOST_ID`，填写上一步返回的 Host id
- `FOSTER_ADB_PATH`
- `FOSTER_MUMU_CLI_PATH`
- `FOSTER_EMULATORS_JSON`

示例：

```dotenv
FOSTER_SERVER_WS_URL=ws://10.0.0.10:8080/agent/ws
FOSTER_AGENT_TOKEN=your-shared-agent-token
FOSTER_AGENT_ID=host-01-agent
FOSTER_HOST_ID=1
FOSTER_OAS_BASE_URL=http://127.0.0.1:22270
FOSTER_ADB_PATH=C:\\Android\\platform-tools\\adb.exe
FOSTER_MUMU_CLI_PATH=C:\\Program Files\\Netease\\MuMu\\nx_main\\mumu-cli.exe
FOSTER_EMULATORS_JSON=[{"emulatorCode":"emu-01","adbSerial":"127.0.0.1:16384","oasConfigName":"oas-emu-01","packageName":null,"startProgram":null,"startArgs":[],"stopProgram":null,"stopArgs":[],"loginPrepareProgram":null,"loginPrepareArgs":[],"loginPrepareDelayMs":0}]
```

### 4.1 先跑宿主机 Smoke

在真正启动 Agent 前执行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\Test-FosterHost.ps1
```

脚本只做只读检查，不启动/停止模拟器，也不会操作游戏。它会检查：

- Server `/readyz`；
- 本地 OAS `/openapi.json`；
- `agent.env` 必填变量；
- `FOSTER_EMULATORS_JSON`；
- ADB executable；
- 每个配置模拟器的 `adb -s <serial> get-state`；
- `emulatorCode / adbSerial / oasConfigName` 是否齐全；
- MuMu CLI（`mumu-cli info` 只读探测）。

只有看到：

```text
Foster host preflight passed.
```

再进入下一步。

针对 MuMu 全渠道登录，再跑一次专用只读预检（可传未填的 `agent.env`，缺失项会给出默认值）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\Test-MumuFullChannel.ps1 -EnvFile .\agent.env
```

它会报告 MuMu 实例列表（索引、启动状态、ADB endpoint）、每个已启动实例的普通包/全渠道包装态、ADB 与 OAS 可达性。全程只读，不卸载、不安装、不启动任何东西。

### 4.2 前台启动 Agent

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\Start-FosterAgent.ps1
```

先观察 Host / Emulator 是否稳定在线。

确认稳定后再安装登录自启：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\Install-FosterAgentTask.ps1 -AgentDir C:\FosterAgent
Start-ScheduledTask -TaskName FosterAgent
```

默认使用**当前 Windows 交互用户**并在该用户登录后启动，不使用 SYSTEM。  
原因是 MuMu/雷电等 GUI 模拟器及其厂商启动命令需要和桌面会话保持一致，放到 Session 0 可能出现模拟器已启动但不可交互的问题。

如果使用专门的宿主机账号，可显式指定：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\Install-FosterAgentTask.ps1 \
  -AgentDir C:\FosterAgent \
  -RunAsUser "MACHINE\foster"
```

### 4.3 本地自行编译（仅开发/排障）

如果确实需要在 Windows 本地重新编译：

```powershell
cd foster-platform
cargo build --release -p foster-agent
```

然后把 `target\release\foster-agent.exe` 覆盖到 `C:\FosterAgent\`。生产宿主机日常部署不要求安装 Rust。

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

查看这台 Host 的模拟器：

```bash
curl http://SERVER:8080/admin/hosts/1/emulators \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN"
```

如果某个模拟器允许绑定的账号数不是默认值，可直接配置，不需要改 SQL：

```bash
curl -X PUT http://SERVER:8080/admin/emulators/1/capacity \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"maxAccountCount":3}'
```

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


## 10. Agent 中断后的人工恢复

Agent 重启、命令丢失或下发结果不确定时，Job 进入 `RECOVERY_REQUIRED`，**不会自动再执行**。PLATFORM 的 `RESERVED` 名额在核实之前继续占用，不会因为 Agent 断线就释放；资源卡自然到期后，ResourcePool 会按原有过期规则回收。

先查询待核实任务：

```bash
curl "http://SERVER:8080/admin/recovery-jobs?limit=50" \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN"
```

管理员必须先查看游戏中实际寄养状态、对应模拟器及 Agent 日志。如果看到了确实成功寄养，记录当前剩余秒数（PLATFORM 必填）：

```bash
curl -X POST http://SERVER:8080/admin/recovery-jobs/JOB_ID/resolve \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "expectedAttempt": 2,
    "action": "CONFIRM_SUCCEEDED",
    "operator": "ops-01",
    "note": "Checked game foster screen; card active",
    "observedRemainingSeconds": 1800
  }'
```

如果确认**从未执行寄养**，而且原 Agent/OAS 执行进程已停止，才能选择重新排队：

```bash
curl -X POST http://SERVER:8080/admin/recovery-jobs/JOB_ID/resolve \
  -H "Authorization: Bearer YOUR_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "expectedAttempt": 2,
    "action": "CONFIRM_NOT_EXECUTED",
    "operator": "ops-01",
    "note": "Checked game and stopped previous OAS process",
    "confirmedStopped": true
  }'
```

此操作会在事务中释放仍处于 RESERVED 的资源名额、增加任务 `retry_count`，并在 5 分钟后重新排队。旧 attempt 的迟到事件不会更新新 attempt。所有人工操作写入 `foster_recovery_audit`；重复操作或错误 attempt 会返回冲突。

**无法确认时，不要选择任何动作。** 保留 `RECOVERY_REQUIRED` 和资源占坑，直到核实或资源卡自然到期。确认成功但资源卡已自然到期的历史异常，不应伪造新的剩余时长。


## 11. MuMu 全渠道扫码登录

登录准备由 Agent 自动编排，流程如下：

1. `mumu-cli info --vmindex all` 查询所有 MuMu 实例；
2. 自动挑选一个空闲实例（排除已占用、ADB 不在线、未启动完成的实例）并加实例锁；
3. 对**所有**发现的实例卸载普通包 `com.netease.onmyoji`；
4. 选中的实例设置分辨率 1280×720；
5. 若实例未启动则通过 `mumu-cli control --vmindex N launch` 启动并等待 ADB 在线；
6. 打开 MuMu 应用市场（`com.mumu.store/.MainActivity`）；
7. 搜索《阴阳师》并选中“全渠道”版本，触发安装；
8. 轮询包管理器，直到全渠道包 `com.netease.onmyoji.wyzymnqsd_cps` 安装完成；
9. 通过 ADB 启动全渠道包（只允许启动这个包名）；
10. `adb exec-out screencap -p` 截取扫码页，经现有 `QR_READY` 事件显示到登录 H5，用户扫码后由 OAS `/login/detect` 识别身份。

### 11.1 破坏性操作警告

登录准备开始时，Agent 会对**所有** MuMu 实例执行 `pm uninstall com.netease.onmyoji`。这会删除普通包在本地的全部数据（登录态、缓存）。全渠道包与普通包数据互不影响；已安装全渠道包的实例不受影响。

### 11.2 硬失败规则

以下情况登录准备直接失败，绝不会回退启动普通包：

- 应用市场没有“全渠道”选项，或安装完成后包名不是 `com.netease.onmyoji.wyzymnqsd_cps`；
- 普通包卸载失败或卸载后仍存在；
- 应用市场 UI 选择器失效（找不到搜索框/游戏条目/安装按钮）或安装超时；
- MuMu CLI 返回坏 JSON、非零退出码或超时。

实例锁在登录会话结束、取消、失败或 Agent 重启时自动释放。

### 11.3 相关配置

| 变量 | 默认值 | 说明 |
|---|---|---|
| `FOSTER_MUMU_CLI_PATH` | `C:\Program Files\Netease\MuMu\nx_main\mumu-cli.exe` | MuMu CLI 路径 |
| `FOSTER_MUMU_APP_MARKET_PACKAGE` | `com.mumu.store` | 应用市场包名 |
| `FOSTER_MUMU_APP_MARKET_ACTIVITY` | `com.mumu.store/.MainActivity` | 应用市场入口 |
| `FOSTER_FULL_CHANNEL_PACKAGE` | `com.netease.onmyoji.wyzymnqsd_cps` | 全渠道包名（不可改为其他值） |
| `FOSTER_NORMAL_PACKAGE` | `com.netease.onmyoji` | 普通包名 |
| `FOSTER_RESOLUTION_WIDTH` / `FOSTER_RESOLUTION_HEIGHT` | `1280` / `720` | 分辨率 |
| `FOSTER_MUMU_COMMAND_TIMEOUT_SECONDS` | `30` | MuMu CLI 命令超时 |
| `FOSTER_MUMU_MARKET_TIMEOUT_SECONDS` | `600` | 应用市场安装总超时 |
| `FOSTER_MUMU_MARKET_POLL_SECONDS` | `2` | 安装轮询间隔 |
| `FOSTER_MUMU_LAUNCH_WAIT_SECONDS` | `120` | ADB 在线/游戏启动等待 |
| `FOSTER_MUMU_LAUNCH_SETTLE_SECONDS` | `5` | 启动后到截图的稳定等待 |

`FOSTER_EMULATORS_JSON` 中 `packageName` 必须保持 `null`：游戏启动包名由 Agent 在包名校验通过后自行决定，配置里的 `adbSerial` 需与 MuMu 实例上报的 endpoint 一致，`oasConfigName` 对应该实例的 OAS 配置名。
