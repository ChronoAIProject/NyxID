# Android 全流程回归测试矩阵 v2

> 版本追加：同步团队 main 后的新 UI、原生 Debug 重建及 2026-11-12 验证见 [共享语音对齐记录](MOBILE_VOICE_PARITY_2026-11-12.md)。本页既有测试计数和截图保留为历史证据，未自动升级为最新版本全流程通过。

本矩阵覆盖当前 React Native Android 应用实际可达的用户路径。2026-11-11 已在真实运行的 Android 模拟器上，使用用户完成登录的会话逐项操作；设备证据和仍未覆盖的子步骤见 [本轮执行报告](ANDROID_RUNTIME_REGRESSION_2026-11-11.md)。聚合用例包含多个子步骤，打开页面或某个子步骤通过，不能把整行标成 `PASS`。

本轮执行工作树为 `codex/android-build-fixes`，开始实测时的提交为 `d633f15f0430de6bf8e78dcb196a8923a3bceb2a`。测试过程中同时修复移动端缺陷，因此应结合执行报告的修复记录和后续提交复核版本，不能将所有截图归为同一个未修改的 APK。

当前 43 项聚合用例的统计为 **1 `PASS`、37 `PARTIAL`、5 `BLOCKED`**。AUTH-01 保留未登录公开入口的完整范围，OPS-01 保留实际应用并清空 Agent 筛选的范围，因此均为 `PARTIAL`。23 个直接测试文件的 365 项检查、语音控制 2 文件 65 项检查及构建环境 1 文件 22 项检查互不重叠，共 26 文件、452 项代码测试通过；不能据此上调设备路径结果。完整文件清单、精确命令与证据边界见执行报告的“聚焦验证记录”。

## 标记和证据约定

| 标记 | 含义 |
| --- | --- |
| `PASS` | 本行当前定义的用户路径已在目标 Android 模拟器上完成，并有对应证据。 |
| `PARTIAL` | 已完成明确列出的子步骤；其余步骤尚未执行、缺少数据，或修复后仍待设备复验。 |
| `BLOCKED` | 关键路径尚无可用或安全的前置条件；如只是待执行，明确注明，不声称设备或 SDK 缺失。 |
| `FAIL` | 仍可复现且未通过设备复验的缺陷。修复过程和阻塞应在执行报告单独保留。 |
| `CODE-PASS` | 聚焦单测、静态检查或配置检查通过；不能代替真实设备验收。 |
| `N/A` | 当前账号、profile 或原生产品契约不提供该能力。 |

截图保存在执行机器的 `/tmp/nyxid-android-runtime/`，下表用截图序号或文件名定位。截图和 UI XML 可能含个人数据，未提交到仓库；临时目录也不是持久化 CI 产物。本轮真实设备由主执行 agent 独占操作，其他 agent 负责独立分析、修复和审查，没有五个 agent 同时控制模拟器。

## 实测环境与前置条件

- AVD：`NyxID_QA_API34`，Pixel 7，Android 14 / API 34，ARM64，`emulator-5554`，1080 × 2400。
- Android SDK：`/Users/abigaildeng/Library/Android/sdk`；JDK 17：Homebrew OpenJDK 17。
- 实测包：`dev.nyxid.nativecheck`，versionName `1.0.4`；Metro 提供当前工作树代码，连接真实后端。
- 已实际完成原生 debug APK 构建、安装和启动，构建日志记录 `BUILD SUCCESSFUL`、464 tasks。旧版报告中“没有 adb/emulator/Java”的判断是发现方式有误，已撤回。
- 用户本人完成 Google 登录。保留当前登录会话；本轮没有登录 Apple。
- 开发包未配置远程 FCM；已有账户资源以只读操作为主。会改变数据的测试使用明确命名的临时 QA Agent、Crew、消息和文件。
- `MODIFY_AUDIO_SETTINGS` 已通过 ARM64 debug 重建、`install -r` 和 `dumpsys package` 确认进入安装包并获准。最终无宿主音频复验中，语音从 Start 进入 Listening 并维持至 3:01 空闲结束，早先约 10 秒控制连接错误未再现；该轮使用 `-no-audio`，真实录音和播放仍未验证。

日常开发可从 `mobile/` 使用 `pnpm run android`，该脚本负责相应 profile 的 clean prebuild。切换 profile 后不能用旧原生目录推断新配置已生效。本轮已经完成的 debug 原生构建属于运行模拟器所需的构建；完整前端套件、全量类型检查和生产构建交 GitHub CI，不能将历史静态检查与 release 验证混为一谈。

## 当前导航基准

- 聊天左上角打开 drawer，常用入口包括 Automations、Machines、Plugins、Activity、Settings，以及 Search、Recents、Scan QR code、Terminal login code、新建 Agent 和 Crews。
- Agent 管理通过聊天身份选择器或 drawer 的 Agent 列表进入；不能假定有旧版的 “Agents & Groups” 页签。
- 群组在当前界面叫 **Crews**；内部路由和代码中的 Group 命名不代表 UI 仍有 Groups tab。
- 服务连接通过 **Plugins → Connectors** 进入；Skills 是 Plugins 的另一个页签。Connector 的 Added/Available 分组及管理页以实际界面为准。
- 原生 Automations 为只读列表、详情和运行记录；创建或更改通过 **Ask NyxBot → 带预填草稿的新聊天**。不存在旧矩阵中的原生 Task → When → Delivery → Review 创建向导，也不要求原生 pause/resume、Run now、edit、delete 控件。
- 原生设置不提供独立模型选择器或完整技能编辑器；测试不得把 Web 专属管理界面当作缺失的 Android 页面。

## 所有路径都应检查的横向规则

Android Back 应优先收键盘、关闭当前内层界面，再返回详情或聊天；快速点击不应重复提交。加载、错误、重试、离线和前后台状态需要分别验证；一次正常网络下的页面打开不能代表这些异常路径通过。安全区域、底部栏、长文本、主题、文字缩放和窗口变化也需要分别留证。

## A. 冷启动、登录与公开入口

| ID | 当前用户路径 | 结果 | 已验证证据与剩余步骤 |
| --- | --- | --- | --- |
| AUTH-01 | 安装 dev 包、启动、splash、公开配置加载、进入登录/工作区 | `PARTIAL` | 音频权限修复后的 ARM64 debug APK 构建、保留数据安装成功；161/197/208 重启后恢复已登录工作区。Google 登录本身由用户完成；未退出会话，最终包未独立复验未登录的公开配置入口。 |
| AUTH-02 | 检查邮箱和服务器启用的社交登录入口、Terms、Privacy | `PARTIAL` | Google 登录由用户完成；已登录法律页 051–052 可打开返回。登录页完整入口在最终包中的再验及登录前法律页未补测；未登录 Apple。 |
| AUTH-03 | 空值、错误邮箱/密码、重复点击登录、错误后重试 | `BLOCKED` | 为保留当前会话，尚未切回未登录状态或准备隔离的错误凭据用例；只有历史代码测试，不能标设备通过。 |
| AUTH-04 | MFA 正确/错误验证码、恢复与最终登录 | `BLOCKED` | 未准备本轮可安全使用的 MFA 挑战及恢复数据，没有实际执行。 |
| AUTH-05 | 已启用社交 provider 的完成、取消、失败和前台回调 | `PARTIAL` | 用户已完成 Google 登录；旧生产包移除后 `nyxid://` 回调只有当前 dev 包接收。GitHub 完整登录、取消/失败组合未测；用户明确不登录 Apple。 |
| AUTH-06 | 登录前后法律页、返回和前后台恢复 | `PARTIAL` | 已登录 Terms/Privacy 051–052 打开并返回；登录前及法律页切后台组合未测。 |
| AUTH-07 | Scan QR、相机拒绝/允许、手动设备码、无效/过期码与审批 | `PARTIAL` | 142–148 已测扫码入口、系统相机拒绝后重试允许、手动输入、无效码校验及 Back。有效/过期真实请求、扫码识别和 approve/deny 缺少测试请求。 |
| AUTH-08 | Terminal login code / Agent Key 选择及生成、兑换、失效 | `PARTIAL` | 149–151 打开 Terminal 入口和 Agent Key 选择后返回；未生成或兑换 code，未执行 approve/deny/过期流程。 |
| AUTH-09 | 401/refresh 失败、退出、缓存清理、杀进程重开 | `PARTIAL` | 196–197 force-stop 后冷启动恢复原登录；183–195 已测离线草稿和联网恢复。尚未执行失效/退出或清除凭据测试。 |

## B. Workspace 与 Assistant

| ID | 当前用户路径 | 结果 | 已验证证据与剩余步骤 |
| --- | --- | --- | --- |
| CHAT-01 | Drawer、Agent chooser、Crews、Plugins、Automations、Activity、Machines、Settings、Search/Recents、QR/Terminal | `PARTIAL` | 已实际访问上述入口，证据分布于 001–157；各页返回已覆盖多条路径。全入口快速重复点击、所有 modal/Back 组合未穷尽。 |
| CHAT-02 | 长聊天滚到中部 → Chat options/Details → 返回 | `PASS` | 154 底部为编号 126–150，155 上移到 111–139；156 打开选项、157 返回，视口仍为 111–139，没有跳到顶部。 |
| CHAT-03 | 短/长/多行文本、键盘、发送、重新进入、草稿 | `PARTIAL` | 014–016 短文本真实发送并回复；153–157 长文本回归；112–113 Crew 多行文本发送。全部私聊多行、草稿切换、重复点击和键盘 Back 组合仍待补齐。 |
| CHAT-04 | Camera/Photos/Files、取消、选择/移除/重选、上传、发送和预览 | `PARTIAL` | 017–023 入口与系统选择器取消；124–128 选中 75-byte QA 文件、移除重选并发送成功；131 预览系统分享面板后 Back，未外部分享。Photos/Camera 成功采集、上传失败/重试、Crew 附件未测。 |
| CHAT-05 | Agent chooser、Details、Title、History、Working context、Permissions、删除确认 | `PARTIAL` | 003–012 已开详情/标题/上下文/历史、取消已有聊天删除；092–096 选择器/管理。158 composer 权限面板、159 SettingsList 权限菜单全部说明完整。181–182 临时 QA 聊天改名保存通过；删除清理见执行报告。上下文 reset 等写操作未全部执行。 |
| CHAT-06 | 回复 streaming、tool activity、Stop、Retry、history refresh | `PARTIAL` | 015–016、113、153 见到真实 running → 完成。Stop/失败 Retry、工具活动和历史刷新组合未完成。 |
| CHAT-07 | acknowledgement、Allow/Deny、过期、recovery-required、技能发布审批 | `BLOCKED` | 本轮 Activity pending/history 为空，缺少安全的待审测试卡片；未对真实请求作批准/拒绝。 |
| CHAT-08 | Composer dictation 与实时 Voice：开始、识别、编辑、发送、取消、前后台 | `PARTIAL` | 024–029 麦克风拒绝后允许；065 无 RecognitionService 的持久提示且草稿保留。067–070/140–141 记录启动错误和恢复，178 的约 10 秒控制错误保留为历史失败。最终 215 Start → 216 Listening 0:08 → 217 Listening 1:18；218 静音时维持 2:47 并收到服务端空闲提醒字幕，219 在 3:01 正常空闲结束，该次未再现早先控制错误。179 Home 终止、180 关闭后输入通过。最终轮为 `-no-audio`，不能证明录音、播放、真实转写或双向语音；dictation 实际识别仍受镜像缺少服务阻塞。 |
| CHAT-09 | History、搜索、Recents、分页、目标聊天、conversation deep link | `PARTIAL` | 011–012 历史/无匹配搜索，152 全局搜索打开 QA chat；真实游标分页、latest 和冷启动 conversation link 未完整覆盖。 |

## C. Agents、Crews、Plugins/Connectors、Machines

| ID | 当前用户路径 | 结果 | 已验证证据与剩余步骤 |
| --- | --- | --- | --- |
| WORK-01 | Agent 管理列表、搜索/清空、筛选、刷新、空/错误/Retry、详情 | `PARTIAL` | 092–096 管理入口、搜索及筛选、详情已操作；服务端错误/Retry 和筛选返回状态组合未全部验证。 |
| WORK-02 | 创建 Agent 实际表单、dirty Back、保存/取消、创建后详情 | `PARTIAL` | 085–088 创建临时 QA Agent，087 dirty Back 选择 Keep editing；创建后进入正确详情。未独立完成所有字段无效值、取消丢弃和快速重复提交组合；不要求不存在的原生模型选择器。 |
| WORK-03 | Agent 改名、权限/连接、Memory、Channels/Threads/Skills/Automations、销毁/删除 | `PARTIAL` | 089 改名保存；090 machine access 空态、091 自动化筛选；101/123 GitHub grant 添加后移除；105–106 Memory 添加后忘记；135–138 QA Agent 销毁→只读→永久删除。122 验证内部 voice 服务已排除。Channels、全部 Skills 操作及冲突 reload 尚未完整覆盖。 |
| WORK-04 | Crews 列表、创建、成员、设置改名、保存、离开/删除 | `PARTIAL` | 107–111 以 QA Agent 创建临时 Crew；114–115 改名持久化；119–120 确认删除。lead/成员无效组合、权限拒绝及 Leave 分支未执行。 |
| WORK-05 | Crew 多行聊天、mention、收件人、附件、待处理 action、设置/返回 | `PARTIAL` | 112–113 多行键盘输入、发送和回复；116–118 mention picker 插入，114–115 设置。附件、pending action 和全部收件人/草稿/键盘组合未完成；不要求已按原生设计省略的投票控件。 |
| WORK-06 | Plugins → Connectors/Skills，Added/Available、搜索/清空、刷新与管理页 | `PARTIAL` | 075–084 两页签、Connector 搜索/清空、GitHub 管理、Skills 详情已打开。185–186 发现离线无限 loading；199 无缓存离线提示、201 重连自动加载、202 离线缓存列表通过。Skills 的 203 双 busy 修复后，222 无缓存分别提示 Added/catalog unavailable offline；223 复网自动取数，224 目录实际加载，225 再断网缓存目录仍可读，226 离线点击没有跳转详情；远端调用 guard 另有聚焦代码测试。所有连接目录边界仍未覆盖。 |
| WORK-07 | Skills 详情、版本、Agent 选择、精确 pin/remove、审批 | `PARTIAL` | 081–084 已打开详情、版本选择后 Back、Agent 选择后 Cancel；225 缓存目录可读，226 离线点击不发远端请求、不跳详情。没有修改现有技能绑定。pin/remove、版本冲突及 authored review 缺少安全测试数据；技能编辑属于 NyxBot。 |
| WORK-08 | Plugins → Connectors → 连接设置，API key/platform/OAuth/Web handoff | `PARTIAL` | 077–080 已有 GitHub 连接管理只读页面可访问。未新建/替换现有凭据，没有完整 provider OAuth 取消/失败/成功验收。 |
| WORK-09 | Connector scopes、history、advanced/proxy、替换/移除/启停 | `PARTIAL` | 078 scopes 展开、079 history、080 advanced 只读已验证。未操作现有连接启停、替换凭据、移除或分享代理数据。 |
| WORK-10 | Machines、Saved logins、详情/Policy/Activity、Agent access、Desktop | `PARTIAL` | 073–074 inventory/Saved logins 空态；090 Agent machine access 空态。账号没有可用测试机器及 saved login，详情、策略写入、Desktop 与在线/离线组合 `BLOCKED`。 |

## D. Automations、Activity、Settings

| ID | 当前用户路径 | 结果 | 已验证证据与剩余步骤 |
| --- | --- | --- | --- |
| OPS-01 | 只读 Automations 列表搜索、status/Agent 筛选、Ask NyxBot 草稿 | `PARTIAL` | 056–060 搜索无匹配后清空、Active 空态→All、Agent picker 打开/取消；064 Ask NyxBot 打开预填新聊天草稿。尚未实际应用 Agent 筛选后再清空；没有运行或修改已有自动化，也不存在旧版原生创建向导。 |
| OPS-02 | Automation detail、run history/detail、目标线程链接、分页 | `PARTIAL` | 061–063 真实详情、运行历史和单次详情已访问；所有 run 状态、分页与目标线程跳转未完整执行。pause/resume、Run now、edit/delete 不属于当前原生界面。 |
| OPS-03 | Activity Pending/History、Active grants、证据、Approve/Deny/revoke | `PARTIAL` | 071 refresh 后 Pending/History 空态，072 Active grants 空态。缺少待审批或可撤销的测试记录，未执行批准、拒绝或撤销。 |
| OPS-04 | FCM 前台、后台/终止态通知点击、重复点击、challenge link | `BLOCKED` | 当前 dev profile 没有远程 FCM 配置及专用挑战记录；不能用普通页面导航替代真实推送验收。 |
| OPS-05 | Account、Notifications/Telegram、Display、Voice、Privacy、Usage、法律页 | `PARTIAL` | 030–052 已逐页打开返回；033 Light→Dark→Automatic 恢复，034 composer 预览；036/037 语言/模式打开取消；204–205 设备中性文案已复验；038 通知已加载、039 Telegram link 打开取消，041 policy 不改。Telegram 实际绑定、远程推送与所有设置写入持久化未执行。 |
| OPS-06 | Name/avatar、Security、MFA/recovery、sessions/broker revoke | `PARTIAL` | 042–048 页面、Name/avatar editor、授权 app/broker 与安全项只读已访问。未保存真实个人资料，未变更 MFA 或撤销任何现有会话。 |
| OPS-07 | Sign out / Delete account 确认、取消、执行/失败、进程重开 | `PARTIAL` | 054 Sign out → Cancel；055 Delete account → Keep my account，均保留会话。196–197 杀进程后重开保留登录；真正退出、删除和注销后的缓存清理未执行。 |

## E. 原生与横向检查

| ID | 当前用户路径 | 结果 | 已验证证据与剩余步骤 |
| --- | --- | --- | --- |
| NATIVE-01 | dev/prod Expo 配置、原生包、Firebase、intent filters、重建 | `PARTIAL` | 已实际构建/安装/运行 dev APK；旧生产包移除后回调归属唯一；历史配置选择测试通过。新增音频 Manifest 权限已实际重建安装并确认 granted；生产 Firebase 和 release 包未验收。 |
| NATIVE-02 | 相机/麦克风/通知允许、拒绝、永久拒绝、系统设置恢复 | `PARTIAL` | 024–029 mic deny→allow；142–145 camera deny→retry→allow。永久拒绝、系统设置回跳、通知权限和真正录音/识别仍有缺项。 |
| NATIVE-03 | Hardware Back：键盘、drawer、menu、sheet、详情、dirty editor、根页面 | `PARTIAL` | 实测标题/详情/历史、文件选择器、QR、Agent dirty Back、分享面板等多层返回。所有输入/根页面/快速重复操作组合未穷尽。 |
| NATIVE-04 | Chat/Agents/Plugins/Activity/Settings 断网 → Retry → 恢复、离线冷启动 | `PARTIAL` | 183 聊天离线禁止发送/保留草稿；187 Activity、188 Settings、190 Home 明确离线；193–195 恢复发送得到回复。199/201/202 Connectors 及 222–226 Skills 无缓存提示、复网自动加载、离线缓存路径已复验。227 离线冷启动为黑屏，228 未获得新的 root dump，离线冷启动仍未通过；debug 包依赖 Metro，现有证据不足以归因为业务层缺陷。没有执行所有页面的失败/Retry 组合。 |
| NATIVE-05 | OAuth、voice、upload、stream、approval、editor 前后台恢复 | `PARTIAL` | 已处理系统 mic 权限 Activity 的临时切换，140–141 不再因权限窗口误判 inactive；179 实际 Home 后立即返回，通话已停止并提示 inactive；180 关闭语音后正常键盘输入。upload/stream/approval/editor 和最终 APK 中的所有组合仍未全测。 |
| NATIVE-06 | challenge/device/Agent Key/auth links 的前台、后台、冷启动及无效参数 | `PARTIAL` | Google auth 回调已成功且当前 scheme 归属唯一；142–151 扫码/无效设备码/Terminal 入口可用。未执行所有 challenge/Agent Key link 的 adb 前后台/冷启动组合。 |
| NATIVE-07 | Phone/large window、横竖屏、长列表、sheet、键盘、安全区域 | `PARTIAL` | 当前 phone 竖屏已覆盖多个长页、键盘与弹层，154–159 验证聊天滚动保持及权限说明完整。横屏/大屏/系统文字缩放的实际设备验证未完成；代码尺寸回归不算设备通过。 |
| NATIVE-08 | 生产 signing、release Gradle/AAB、分发后验证 | `BLOCKED` | 本轮只有 debug 原生构建；release signing/AAB/分发没有执行，交对应 CI 与发布环境验收。 |

## 历史五 agent 代码检查：保留记录，不计为设备通过

以下分支均从固定基线 `0246eebd3d587fc4805db09371ba8d36dd5dee41` 创建；它们是先前的代码/契约检查，不是本轮模拟器执行结果。先前从 PATH 未发现工具就写成“没有 adb/emulator/Java”是错误结论；本轮已找到 SDK/JDK 并实际运行模拟器。

| Agent | 分支 | 历史聚焦结果 | 能证明的范围 |
| --- | --- | --- | --- |
| 1 | `codex/android-qa-auth` | 201 通过；扩展选择集 240 通过 | 认证、OAuth/MFA/session/导航模型；不能证明第三方实际登录。 |
| 2 | `codex/android-qa-chat` | 942 通过 | Assistant/composer/附件/voice/drawer 状态；不能证明原生录音、键盘和实际滚动。 |
| 3 | `codex/android-qa-workspace` | 329 通过；额外导航选择集 377 通过 | Agents/Crews/Plugins/Services/Machines contract；不能证明真实 provider/机器执行。 |
| 4 | `codex/android-qa-ops` | 166 通过 | Automations/Activity/Account/Security 状态；不能证明 FCM、MFA 或真实账户销毁。 |
| 5 | `codex/android-qa-native` | 65 通过 | Expo/profile/Firebase/signing/linking 静态契约；不能证明 release 构建和设备权限。 |

扩展选择集可能包含前一个选择集，不能将这些数字简单相加当作独立用例总数。本轮修复后的聚焦检查、设备证据和剩余阻塞另见执行报告。当前禁止把这张矩阵概括成“Android 全流程全部通过”。
