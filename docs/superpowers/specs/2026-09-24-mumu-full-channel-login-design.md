# MuMu 全渠道扫码登录编排设计

## 目标

将寄养登录流程从“使用已在线模拟器并启动固定包名”升级为可自助准备运行环境的 MuMu 编排流程：自动选择空闲 MuMu 实例，统一设置分辨率，通过 MuMu 应用市场安装并确认全渠道《阴阳师》，启动游戏后截取扫码登录页，最后把二维码交给用户。

## 已确认需求

- 模拟器：MuMu。
- 每次登录自动选择一个空闲实例，不固定使用实例 0。
- 分辨率：1280×720。
- 只使用全渠道包：`com.netease.onmyoji.wyzymnqsd_cps`。
- 普通包：`com.netease.onmyoji`；启动流程开始前清理所有 MuMu 实例中的普通包。
- 全渠道包通过 MuMu 应用市场选择“全渠道”版本安装，不依赖本地 APK 路径。
- 安装结果必须通过包名校验；无法确认全渠道包时停止，不启动普通包。
- 登录方式为二维码扫码；二维码由模拟器截图生成并显示在登录 H5。

## 当前实现缺口

当前 `GenericAdbEmulatorDriver` 只负责等待 ADB、调用 `monkey` 启动指定包和截图；`EmulatorInstanceConfig` 仅有可选的外部启动命令，不能发现 MuMu 实例、设置分辨率、操作 MuMu 应用市场或清理错误包。Agent 的登录状态机和 Server 的登录 H5 可以复用，但 Agent 需要增加准备阶段与失败事件。

## 设计

### 1. MuMu 控制适配器

新增 MuMu 控制适配层，封装 `mumu-cli.exe`：

- `info --vmindex all` 获取实例索引、名称、启动状态、ADB 地址和端口。
- `control --vmindex N launch` 启动实例。
- `setting --vmindex N --key resolution_width.custom --value 1280 --key resolution_height.custom --value 720` 设置分辨率。
- `control --vmindex N app ...` 或 ADB 方式打开应用市场及执行基础应用控制。
- 所有外部命令使用结构化参数，不拼接用户输入；记录命令阶段、实例索引和返回码，不记录令牌。

MuMu CLI 路径作为 Agent 配置项，默认使用本机安装路径；路径不存在时 Agent 在 Host 心跳/准备阶段报告明确错误。

### 2. 空闲实例选择

Agent 维护每个 MuMu 实例的本地准备锁。选择候选时排除：已有活动登录/寄养命令、ADB 不在线、MuMu 正在启动或准备失败的实例。按实例索引稳定排序选择第一个空闲候选，准备锁从启动准备阶段持有到登录会话完成、取消或失败。

Server 仍负责容量和账号绑定；Agent 只负责本机实例调度，不能绕过 Server 的 Host/Emulator 分配。

### 3. 包清理与全渠道安装

登录准备开始时，遍历 MuMu 实例并执行：

1. 查询 `com.netease.onmyoji` 和 `com.netease.onmyoji.wyzymnqsd_cps` 的安装状态。
2. 对普通包执行卸载；卸载失败立即记录实例级失败并停止本次准备。
3. 对选中实例打开 MuMu 应用市场，搜索《阴阳师》，选择“全渠道”版本并触发安装。
4. 轮询 ADB 包管理器，直到全渠道包出现且安装完成。
5. 若应用市场没有全渠道选项、安装超时、包名不匹配或普通包仍存在，返回不可恢复的准备失败，不调用游戏启动。

应用市场操作需要独立的 UI 自动化步骤/选择器，选择器失效时必须报告“无法确认全渠道版本”，禁止按默认位置盲点安装。

### 4. 游戏启动与扫码截图

在全渠道包校验通过后：

- 使用选中实例的 ADB serial 启动 `com.netease.onmyoji.wyzymnqsd_cps`。
- 等待应用进入登录界面；OAS `/login/detect` 只负责二维码后的账号识别，不负责启动游戏。
- 通过 `adb exec-out screencap -p` 获取登录二维码截图。
- Server 继续使用现有 `QR_READY` 状态和 H5 数据接口；二维码过期、扫码后身份识别、用户确认沿用现有状态机。

### 5. 配置接口

扩展 Agent 环境/配置字段：

- `FOSTER_MUMU_CLI_PATH`
- `FOSTER_MUMU_APP_MARKET_PACKAGE`（MuMu 应用市场包名或启动方式）
- `FOSTER_FULL_CHANNEL_PACKAGE`，默认 `com.netease.onmyoji.wyzymnqsd_cps`
- `FOSTER_NORMAL_PACKAGE`，默认 `com.netease.onmyoji`
- `FOSTER_RESOLUTION_WIDTH=1280`
- `FOSTER_RESOLUTION_HEIGHT=720`
- 应用市场选择器/等待超时配置

现有 `FOSTER_EMULATORS_JSON` 保留为 OAS 配置名和实例标识映射，不再要求调用方预先把实例启动好；实例的动态 ADB serial 由 MuMu 查询结果更新到运行时映射。

## 错误与安全边界

- 不允许启动普通包；包名校验失败是硬失败。
- 卸载普通包会删除该包的本地数据，流程日志必须明确记录影响范围。
- MuMu CLI、ADB、应用市场、安装和游戏启动每一步都有超时、返回码检查和可读错误。
- 登录令牌、Agent token、二维码原始图片只通过现有受控接口传递，不写入普通日志。
- Agent 重启或准备中断时释放本地准备锁，并让 Server 进入现有恢复/重试路径。

## 测试与验收

- 单元测试：MuMu CLI 输出解析、实例空闲选择、包名校验、分辨率参数生成、失败状态映射。
- 集成测试：模拟 MuMu CLI/ADB/OAS，覆盖启动、普通包清理、全渠道包存在、安装超时、错误包拒绝和二维码准备。
- 真机验收：两个 MuMu 实例一开一闲时选择闲置实例；设置后读取分辨率为 1280×720；普通包被清理；应用市场安装全渠道包；游戏启动；登录 H5 显示二维码；扫码后能进入账号确认。

