# 阴阳师寄养托管平台设计规格

- 日期：2026-09-21
- 状态：Design approved in conversation; awaiting written-spec review
- 仓库：`shenliuming/OnmyojiAutoScript`
- 分支：`feat/foster-platform`
- 目标：在保留 OAS 游戏自动化能力的前提下，增加可商业运营的寄养托管平台
- 技术方向：Rust Server + Rust Agent + 现有 Python OAS

## 1. 产品目标

平台提供两类寄养服务。

### 1.1 基础自动寄养

- 参考价格：约 10 元/月。
- 每日目标约 4 次。
- 默认每次成功后约 6 小时进入下一轮。
- 使用客户自己的好友列表和已有结界卡资源。
- 平台负责自动切号、自动寄养、失败处理、寄养记录和账号保护。

### 1.2 平台资源寄养

- 参考价格：约 50 元/月，实际价格由套餐配置决定。
- 在基础寄养能力之上，平台提供斗鱼或太鼓/勾玉资源。
- Server 统一管理平台资源号、卡周期、坑位和客户可使用的资源。
- MVP 阶段资源号与客户之间的好友关系由运营人工建立。
- 自动加好友放到后续阶段。

### 1.3 用户自助能力

用户通过专属 URL 使用 H5：

- 查看服务状态。
- 查看今日成功次数。
- 查看最近寄养记录和成功截图。
- 查看下次预计执行时间。
- 配置固定“不上号”时段。
- 临时点击“我要玩游戏”，暂停 1/2/4 小时或当天暂停。
- 登录失效时重新扫码。
- 首次购买后通过网页显示的游戏二维码完成扫码登录。

## 2. 设计边界

系统拆为三个明确边界。

### 2.1 Rust Server

负责全部业务权威状态：

- 客户和游戏账号。
- 套餐与订阅。
- Host / Agent / Emulator。
- 模拟器账号容量。
- 游戏账号与模拟器绑定。
- 登录会话。
- 静默时段和临时暂停。
- Scheduler。
- FosterJob。
- 平台资源池。
- 寄养记录。
- 用户查询链接。
- Server ↔ Agent 状态对账。

### 2.2 Rust Agent

每台 Windows Host 运行一个 Agent，负责：

- 主动连接 Server WebSocket。
- 发现和管理模拟器实例。
- 模拟器启动、停止、截图、ADB。
- 一个模拟器内多个账号的切换。
- 调用 OAS 完成一次具体动作。
- 截取登录二维码。
- 上报命令状态、执行阶段和结果。
- 本地命令幂等和断线恢复。

### 2.3 OAS / Python

继续负责游戏自动化：

- 页面识别。
- 点击。
- OCR。
- 账号切换。
- 角色 / 区服 / 脱敏账号识别。
- 执行一次寄养。
- 基础版自动扫描客户好友。
- 平台版进入 Server 指定的资源好友。
- 返回结果、错误码、剩余时间和截图。

OAS 不承担：

- 套餐。
- 收费。
- 订单。
- 全局 6 小时调度。
- 静默时段。
- 资源池调度。
- 模拟器容量。
- 用户 H5。

## 3. 总体架构

```text
用户 H5
  │ HTTPS / SSE
  ▼
Rust Server
  ├─ Public API
  ├─ Login Service
  ├─ Subscription Service
  ├─ Scheduler
  ├─ Job Dispatcher
  ├─ Resource Pool
  ├─ Agent Gateway
  │
  ├─ MySQL
  └─ Redis
        │
        │ WSS
        ▼
Windows Host
  └─ Rust Agent
       ├─ EmulatorDriver
       ├─ Command Journal
       ├─ OAS Bridge
       └─ Reconciliation
            │
            ▼
       Emulator Instance
            │
            ▼
          OAS/Python
            │
            ▼
          阴阳师
```

MVP 可以只部署一个 Host，但所有表和协议都必须天然支持 Host-02、Host-03 横向扩展。

## 4. Rust Workspace

建议在当前仓库新增：

```text
foster-platform/
├── Cargo.toml
├── crates/
│   ├── server/
│   │   └── src/
│   │       ├── api/
│   │       ├── scheduler/
│   │       ├── dispatcher/
│   │       ├── services/
│   │       ├── repository/
│   │       └── main.rs
│   ├── agent/
│   │   └── src/
│   │       ├── emulator/
│   │       ├── command/
│   │       ├── oas/
│   │       ├── ws/
│   │       └── main.rs
│   ├── domain/
│   │   └── src/
│   │       ├── account.rs
│   │       ├── emulator.rs
│   │       ├── login.rs
│   │       ├── subscription.rs
│   │       ├── job.rs
│   │       └── resource.rs
│   └── protocol/
│       └── src/
│           ├── server_message.rs
│           ├── agent_message.rs
│           └── types.rs
├── migrations/
├── web/
└── docker/
```

推荐依赖：

- axum
- tokio
- sqlx + MySQL
- redis
- serde / serde_json
- chrono
- uuid
- tracing
- reqwest
- tower-http
- async-trait

## 5. Host 与 Agent

### 5.1 Host

核心字段：

- id
- host_code
- hostname
- status: ONLINE / OFFLINE / MAINTENANCE
- agent_version
- last_heartbeat_at

### 5.2 Agent 连接

Agent 主动连接：

`wss://<server>/agent/ws`

Windows 主机不要求公网 IP 和入站端口。

首版认证使用 Agent Token。

Agent 启动时发送 `AgentHello`：

- agent_id
- host_id
- agent_version
- hostname
- os_version
- capabilities

Server 返回：

- heartbeat_interval_secs
- server_time

默认心跳 15 秒。

约 45 秒未收到心跳：

- Host 标记 OFFLINE。
- Emulator 标记 UNKNOWN/OFFLINE。
- 未开始的 Job 不再下发。
- 正在执行的 Job 不立即判失败。

Agent 重连必须做状态对账。

## 6. 模拟器模型

### 6.1 EmulatorInstance

字段：

- id
- host_id
- emulator_code
- driver_type: UNKNOWN / MUMU / LDPLAYER / OTHER
- max_account_count
- status
- adb_serial
- current_job_id
- last_heartbeat_at

状态：

- OFFLINE
- IDLE
- SWITCHING_ACCOUNT
- RUNNING
- LOGIN_SESSION
- MAINTENANCE
- ERROR

### 6.2 EmulatorDriver 抽象

首版不绑定 MuMu 或雷电。

```rust
#[async_trait]
pub trait EmulatorDriver {
    async fn list_instances(&self) -> Result<Vec<EmulatorInstance>>;
    async fn start(&self, instance_id: &str) -> Result<()>;
    async fn stop(&self, instance_id: &str) -> Result<()>;
    async fn adb_serial(&self, instance_id: &str) -> Result<String>;
    async fn screenshot(&self, instance_id: &str) -> Result<Vec<u8>>;
    async fn health_check(&self, instance_id: &str) -> Result<EmulatorHealth>;
}
```

确定具体模拟器后，再实现 `MumuDriver` 或 `LdPlayerDriver`。

### 6.3 一个模拟器多个账号

一个模拟器可以保存多个已登录账号，但任意时刻只能执行一个控制任务。

例如：

```text
Emulator-01 max_account_count = 5
├─ slot 1 Account-A ACTIVE
├─ slot 2 Account-B ACTIVE
├─ slot 3 Account-C ACTIVE
├─ slot 4 Account-D PENDING
└─ slot 5 empty
```

容量占用必须计算：

- ACTIVE
- PENDING
- MIGRATING_IN

不持久化容易漂移的 current_account_count 作为事实源。

## 7. 游戏账号与模拟器绑定

### 7.1 核心约束

同一个 GameAccount 任意时刻只允许一个 ACTIVE 模拟器绑定。

正常情况下账号固定归属模拟器，不允许调度器随意搬迁。

### 7.2 Binding 状态

- PENDING
- ACTIVE
- MIGRATING
- UNBOUND

流程：

```text
首次扫码
PENDING
  ↓ 验证成功
ACTIVE

迁移：
ACTIVE
  ↓
MIGRATING
  ├─ 新模拟器验证成功 → 新 ACTIVE / 旧 UNBOUND
  └─ 新模拟器验证失败 → 回滚旧 ACTIVE
```

数据库建议：

- `game_account.active_emulator_id` 保存当前权威绑定。
- `emulator_account_binding` 保存完整历史、slot 和迁移记录。
- 切换绑定必须在事务内完成。

## 8. 账号身份识别

一个模拟器存在多个登录账号，不能只靠脱敏手机号或邮箱判定身份。

### 8.1 Identity 类型

`game_account_identity` 支持：

- MASKED_ACCOUNT
- OCR_ALIAS
- CHARACTER_NAME
- SERVER_NAME
- GAME_UID

每个身份因子保存：

- identity_value
- normalized_value
- source
- confidence
- last_seen_at
- enabled

### 8.2 信任排序

1. GAME_UID：最高。
2. CHARACTER_NAME + SERVER_NAME：高。
3. MASKED_ACCOUNT：中。
4. OCR_ALIAS：辅助。

脱敏账号不是唯一主键。

### 8.3 复用现有 OAS

OAS 当前 `tasks/Component/SwitchAccount` 已具备：

- account
- account_alias
- character
- svr
- `is_account_alias()`

并已经处理邮箱 `@` OCR 易错问题。

平台版不重写这套基础能力，而是由 Server 下发身份描述，Agent 转换为 OAS 所需的 `AccountInfo`。

### 8.4 执行前校验

任何寄养任务必须先做身份校验。

不同账号：

```text
SwitchAccount
→ 脱敏账号 / OCR alias
→ 角色名
→ 区服
→ Game UID（可获取时）
→ 验证通过
→ 执行寄养
```

当前已是目标账号时可以轻量校验，但不能完全跳过。

出现以下情况：

```text
脱敏账号匹配
但角色 / 区服 / UID 不一致
```

立即进入：

`IDENTITY_MISMATCH`

禁止自动重试和继续寄养。

## 9. 套餐与订阅

### 9.1 Plan

套餐配置化。

#### BASIC_AUTO_FOSTER

- resource_mode = USER_FRIEND
- daily_target_runs = 4
- interval_minutes = 360
- 参考售价约 10 元/月

#### PLATFORM_RESOURCE_FOSTER

- resource_mode = PLATFORM
- resource_type = FISH 或 TAIKO_JADE
- daily_target_runs = 4
- interval_minutes = 360
- 参考售价约 50 元/月

### 9.2 Subscription

保存套餐快照：

- game_account_id
- plan_id
- resource_mode
- resource_type
- daily_target_runs
- interval_minutes
- price_snapshot
- start_at
- end_at
- status
- last_success_at
- next_run_at
- manual_pause_until

状态：

- PENDING_LOGIN
- ACTIVE
- PAUSED
- EXPIRED
- SUSPENDED

支付不是 MVP 第一阶段必需能力。首版允许运营后台手工开通、续费和停用。

## 10. 静默时段与“我要玩游戏”

### 10.1 QuietPeriod

一个账号可以配置多个固定时段：

- game_account_id
- weekday_mask
- start_time
- end_time
- timezone
- before_buffer_minutes
- after_buffer_minutes
- enabled

MVP 默认 `Asia/Shanghai`。

例如：

```text
20:00 - 23:00 不上号
before_buffer = 5
after_buffer = 5
```

实际保护窗口：

```text
19:55 - 23:05
```

### 10.2 临时暂停

用户 H5 支持：

- 暂停 1 小时
- 暂停 2 小时
- 暂停 4 小时
- 今天不再上号

Subscription 保存：

`manual_pause_until`

### 10.3 优先级

```text
账号安全异常
>
人工暂停
>
静默时段
>
正常寄养任务
```

安全优先于“每日约 4 次”。

## 11. LoginSession

LoginSession 独立于 FosterJob。

状态：

- CREATED
- WAITING_EMULATOR
- PREPARING
- WAITING_QR
- QR_READY
- WAITING_SCAN
- DETECTING_LOGIN
- VERIFYING_ACCOUNT
- SUCCESS
- QR_EXPIRED
- FAILED
- CANCELLED

### 11.1 首次扫码流程

```text
创建客户 / GameAccount / Subscription
→ 创建 LoginSession
→ 选择有容量 Emulator
→ 创建 PENDING Binding，占用 slot
→ 下发 START_LOGIN
→ Agent 启动模拟器
→ 进入游戏二维码页面
→ 截取 QR 区域
→ 上传短期对象
→ LOGIN_QR_READY
→ H5 展示 QR
→ 用户扫码
→ Agent 检测登录成功
→ 读取脱敏账号 / 角色 / 区服 / UID
→ VERIFYING_ACCOUNT
→ H5 展示角色和区服
→ 用户确认
→ Binding ACTIVE
→ GameAccount VERIFIED / LOGGED_IN
→ Subscription ACTIVE
→ LoginSession SUCCESS
```

### 11.2 QR 安全

- QR 图片只短期存储。
- 建议签名 URL 2~5 分钟。
- DB 保存 object_key + expire_at。
- QR 过期后 Agent 刷新。
- LoginSession 长时间无人完成时释放 PENDING slot。

### 11.3 Browser 状态更新

Browser → Server 使用 HTTPS。

Server → Browser 使用 SSE 推送：

- PREPARING
- QR_READY
- WAITING_SCAN
- VERIFYING_ACCOUNT
- SUCCESS
- FAILED

用户页面不需要额外 WebSocket。

## 12. Server ↔ Agent 协议

### 12.1 ServerCommand

```rust
pub enum ServerCommand {
    StartLogin(StartLoginCommand),
    RunFoster(RunFosterCommand),
    StopCurrentJob(StopCurrentJobCommand),
    StopGame(StopGameCommand),
    RefreshEmulators(RefreshEmulatorsCommand),
    Ping(PingCommand),
}
```

### 12.2 AgentEvent

```rust
pub enum AgentEvent {
    Hello(AgentHello),
    Heartbeat(Heartbeat),
    CommandAccepted(CommandAccepted),
    CommandRejected(CommandRejected),

    LoginPreparing(LoginPreparing),
    LoginQrReady(LoginQrReady),
    LoginQrExpired(LoginQrExpired),
    LoginSucceeded(LoginSucceeded),
    LoginFailed(LoginFailed),

    JobStarted(JobStarted),
    JobStageChanged(JobStageChanged),
    JobFinished(JobFinished),

    EmulatorStatusChanged(EmulatorStatusChanged),
    AccountIdentityDetected(AccountIdentityDetected),
    Pong(Pong),
}
```

### 12.3 命令幂等

每条 ServerCommand 必须带唯一 `command_id`。

Agent 本地 Command Journal 记录：

- RECEIVED
- RUNNING
- FINISHED

重复 command_id：

- RUNNING：返回当前阶段，不重新执行。
- FINISHED：返回缓存结果。
- 禁止重复寄养。

### 12.4 重连对账

Agent 重连发送本地 running_jobs。

Server 与 Agent 对账：

- 双方都认为 RUNNING：继续。
- Server RUNNING、Agent 无任务：Job → RECOVERY_REQUIRED。
- Agent 有任务、Server 已结束：Agent 停止后续业务动作并上报。

## 13. FosterJob

状态：

- PENDING
- DEFERRED_QUIET
- DEFERRED_MANUAL
- WAITING_EMULATOR
- WAITING_RESOURCE
- SWITCHING_ACCOUNT
- VERIFYING_ACCOUNT
- RUNNING
- SUCCESS
- RETRY
- FAILED
- IDENTITY_MISMATCH
- CANCELLED
- RECOVERY_REQUIRED

流程：

```text
PENDING
├─ manual pause → DEFERRED_MANUAL
├─ quiet period → DEFERRED_QUIET
└─ executable
     ↓
WAITING_EMULATOR
     ↓
SWITCHING_ACCOUNT
     ↓
VERIFYING_ACCOUNT
     ├─ mismatch → IDENTITY_MISMATCH
     └─ verified
          ↓
          ├─ BASIC → RUNNING
          └─ PLATFORM
                ↓
          WAITING_RESOURCE
                ↓
            RUNNING
                ↓
       SUCCESS / RETRY / FAILED
```

## 14. Scheduler

Scheduler 不直接控制模拟器，只负责制造 FosterJob。

逻辑：

```text
Subscription.next_run_at 到期
→ 生成 FosterJob
→ 交给 Job Dispatcher
```

建议扫描周期 10~30 秒。

为避免重复创建：

- 订阅进入待执行状态后，必须通过事务或状态字段抢占。
- 同一个计划周期只能生成一个有效 FosterJob。

### 14.1 每日约 4 次规则

不是固定 00/06/12/18。

规则为：

- 成功后，下一次通常 = success_at + interval_minutes。
- 默认 interval_minutes = 360。
- 静默、人工暂停、排队、资源等待均允许顺延。
- 不进行“短时间连续补次数”。

例如：

```text
14:30 成功
→ 理论下一次 20:30
→ 用户 20:00-23:00 静默
→ 顺延至 23:05
→ 23:05 成功
→ 下一次 05:05
```

今日统计只计 SUCCESS。

## 15. Job Dispatcher 与并发

### 15.1 账号锁

同一 GameAccount 永远只能有一个控制任务：

`lock:account:{game_account_id}`

### 15.2 模拟器锁

同一 Emulator 同时只允许一个控制任务：

`lock:emulator:{emulator_id}`

### 15.3 排队

例如同一模拟器：

```text
A 20:00
B 20:01
C 20:02
```

执行：

- A RUNNING
- B WAITING_EMULATOR
- C WAITING_EMULATOR

优先级 MVP：

1. 延迟时间最长。
2. scheduled_at 最早。

### 15.4 二次静默检查

创建 Job 时检查一次。

真正 Dispatch 前必须再检查一次。

如果排队期间进入静默区：

`WAITING_EMULATOR → DEFERRED_QUIET`

禁止启动游戏。

## 16. 切号优化

Agent 可维护当前模拟器最近已验证的 `current_game_account_id`。

- 当前就是目标账号：轻量身份校验后执行。
- 当前不是目标账号：SwitchAccount + 强身份校验。

Server 不完全信任 Agent 的 current account，只把它当优化信息。

## 17. 基础寄养模式

`resource_mode = USER_FRIEND`

流程：

```text
切换目标客户账号
→ 身份校验
→ 打开好友寄养
→ 使用现有 OAS 规则扫描
→ 找可用结界
→ 寄养
→ 返回结果
```

Server 不要求 MVP 阶段知道具体好友昵称。

## 18. 平台资源模式

`resource_mode = PLATFORM`

OAS 不允许自由选择目标资源号。

Server 必须先分配一个 provider/resource cycle。

### 18.1 ProviderAccount

平台维护资源号：

- provider_code
- game_uid
- nickname
- server_name
- status

### 18.2 FriendBinding

MVP 人工加好友后记录：

- game_account_id
- provider_account_id
- status
- verified_at

### 18.3 ResourceCycle

代表某资源号当前一轮结界卡：

- provider_account_id
- resource_type: FISH / TAIKO_JADE
- resource_level
- start_at
- end_at
- slot_capacity
- occupied_slots
- status

### 18.4 ResourceAllocation

状态：

- RESERVED
- CONFIRMED
- RELEASED
- EXPIRED

流程：

```text
Job
→ 过滤客户已经是好友的 Provider
→ 过滤目标资源类型
→ 过滤有效 ResourceCycle
→ 过滤有空位
→ 事务 Reserve
→ 创建 Allocation
→ Agent/OAS 去指定 Provider
→ 游戏内成功 → CONFIRMED
→ 失败 → RELEASED
```

资源坑位必须通过数据库条件更新或事务锁保证并发正确性。

### 18.5 最低剩余时间

资源卡临近结束时不再分配新用户。

配置：

`min_remaining_minutes`

MVP 可先设约 330 分钟，后续运营可调整。

### 18.6 资源排序

MVP：

1. end_at 最早优先。
2. occupied_slots 较高优先。

目标是先消耗即将到期的资源卡。

## 19. 失败与重试策略

错误必须结构化，不统一使用一个“FAILED”。

示例：

### NO_SLOT

- 当前 Provider 坑位被抢。
- RELEASE 当前 Allocation。
- 尝试其他 Provider。
- 可立即重试。

### PROVIDER_NOT_FOUND

- 标记 FriendBinding 疑似异常。
- 换 Provider。
- 后台产生运营提示。

### ACCOUNT_LOGIN_EXPIRED

- 停止寄养自动重试。
- GameAccount → RELOGIN_REQUIRED。
- H5 提示重新扫码。

### IDENTITY_MISMATCH

- 高风险错误。
- 不自动重试。
- 停止该账号后续任务，等待人工处理。

### EMULATOR_OFFLINE

- 等 Host 恢复。
- 不立即判最终失败。

### NETWORK_ERROR

- 5~10 分钟后重试。

### GAME_BUSY

- 短延迟重试。

Job 需要：

- error_code
- result_message
- retry_count
- retry_after
- screenshot_url

## 20. 用户查询链接

不使用可枚举的 `orderId` 暴露查询。

外部 URL：

`https://<host>/r/<random-token>`

设计：

- token 使用安全随机值。
- DB 保存 token hash。
- 支持重新生成，旧 token 失效。
- 查询和控制权限可拆分。

建议：

- 纯查看：长期 token。
- 修改静默时段、暂停上号：增加服务码/手机号/微信等二次验证，MVP 可以先使用独立 control token。

页面展示：

- 服务状态。
- 套餐类型。
- 资源类型。
- 服务到期时间。
- 今日成功 X/4。
- 最近寄养记录。
- 下次预计。
- 当前是否处于静默/暂停状态。
- “我要玩游戏”入口。
- 固定不上号时段设置。
- 重新扫码入口。

## 21. 数据库核心表

MVP 预计至少：

- customer
- game_account
- game_account_identity
- host
- emulator_instance
- emulator_account_binding
- foster_plan
- foster_subscription
- foster_quiet_period
- login_session
- foster_job
- provider_account
- foster_friend_binding
- foster_resource_cycle
- foster_resource_allocation
- foster_share_link

具体字段和索引在 implementation plan 中拆 migration。

## 22. OAS 改造点

当前 OAS 已存在：

- `tasks/KekkaiUtilize`
- `tasks/Component/SwitchAccount`

MVP 原则：小改 OAS，大部分新业务放 Rust。

需要补的 OAS 能力：

1. 单次寄养执行入口。
2. 结构化执行结果。
3. 平台模式按指定 Provider 寻找好友，而不是自动选最优资源。
4. 账号识别结果结构化返回。
5. 可被 Agent 调用的稳定桥接方式。
6. 登录二维码页面定位与二维码区域截图能力，如现有代码不足则补齐。

不要把 Server Scheduler 再塞回 OAS。

## 23. MVP 非目标

首版明确不做：

- 在线支付闭环。
- 自动加资源好友。
- 自动购买/补充结界卡。
- 多种模拟器厂商同时实现。
- 复杂会员等级和营销系统。
- 客户账号密码保存。
- 手机号/邮箱明文作为唯一身份。
- 微服务拆分。
- Kafka / MQ。
- Kubernetes。
- 自动迁移账号到其他 Host。
- AI 异常判断。

## 24. MVP 验收标准

### 24.1 登录

- 后台创建客户与订阅。
- 系统分配有容量的模拟器槽位。
- 用户打开链接看到游戏扫码二维码。
- 用户扫码后能识别角色和区服。
- 用户确认后完成 ACTIVE 绑定。
- 一个 GameAccount 不会出现两个 ACTIVE 模拟器绑定。

### 24.2 多账号模拟器

- 一个模拟器可以绑定多个客户账号。
- max_account_count 生效。
- PENDING login 也占容量。
- 同一个模拟器不并发执行两个任务。

### 24.3 账号安全

- 任务执行前确认目标账号。
- 可识别脱敏账号和 OCR alias。
- 角色/区服/UID 冲突时停止执行。
- 不会在身份不明确时继续寄养。

### 24.4 调度

- 成功后约 6 小时生成下一轮。
- 同一账号不并发。
- 同一模拟器串行。
- 静默时段不会启动游戏。
- “我要玩游戏”能阻止新的任务下发。
- 顺延后不会短时间连续补任务。

### 24.5 BASIC

- 能使用客户自身好友池完成寄养。
- 记录成功/失败。
- 今日成功次数统计正确。

### 24.6 PLATFORM

- Server 可以分配指定 Provider。
- 资源坑位不会被并发超卖。
- 成功后 Allocation CONFIRMED。
- 失败后正确释放。
- 资源临近过期时不会再分配。

### 24.7 用户页面

- 可以通过安全 URL 查看状态和历史。
- 能查看下次预计时间。
- 能设置不上号时段。
- 能临时暂停。
- 登录失效时能重新扫码。

## 25. 开源边界

当前 OAS 采用 GPL-3.0。商业平台必须保持业务层与 OAS 执行层边界清晰。

本设计不试图规避许可证义务。

实施前需进一步确认：

- 对修改后的 OAS 的分发方式。
- Agent 与 OAS 的耦合方式。
- 是否向客户分发任何 GPL 程序或二进制。
- 游戏服务条款与商业托管相关风险。

Rust Server 的商业业务代码应尽量保持独立，不直接复制 OAS 源码实现。

## 26. 已确认的关键决策

- Rust Server + Rust Agent。
- OAS 保持 Python。
- 第一版单 Host，但天然支持多 Host。
- 模拟器厂商暂未确定，使用 EmulatorDriver 抽象。
- Agent → Server 主动 WebSocket 长连接。
- Browser 登录状态使用 SSE。
- 一个模拟器可绑定多个账号，有容量上限。
- 一个 GameAccount 任意时刻只能一个 ACTIVE Emulator 绑定。
- 脱敏账号不能作为唯一身份，使用多因子身份识别。
- 平台资源好友关系 MVP 人工建立。
- BASIC 与 PLATFORM 共用调度系统。
- 静默时段和人工暂停优先于每日次数。
- Server 是唯一业务权威状态源。
- Scheduler 只创建 Job，Dispatcher 决定何时真正下发。
- Server 指定 PLATFORM Provider，OAS 不自行挑选。
- 主分支保持适合继续同步上游，商业改造集中在独立 feature branch。
