# Android 全流程回归测试矩阵 v1

这份矩阵覆盖当前 React Native Android 应用中实际可达的用户路径。它用于真机/模拟器手工验收，也用于把代码测试能够证明的部分和必须依赖真实设备、后端数据或第三方配置的部分分开记录。

测试范围以当前 `mobile/src/app/AppNavigator.tsx`、`mobile/src/app/navigationTypes.ts` 和 `mobile/src/components/workspace/WorkspaceDrawer.tsx` 为准。每个用例都要记录设备型号、Android 版本、应用 commit、账号/数据前置条件、执行时间、结果和截图或日志证据。没有设备或外部配置时使用 `BLOCKED`，不能把未执行写成通过。

## 结果标记

| 标记 | 含义 |
| --- | --- |
| `PASS` | 在目标 Android 设备上完成，结果符合预期，并保存了证据。 |
| `FAIL` | 可以稳定复现的功能或回归问题，记录步骤、日志和截图。 |
| `BLOCKED` | 缺少设备、后端数据、第三方凭据、Firebase/推送配置或其他前置条件。 |
| `CODE-PASS` | 聚焦单测、静态检查或 Expo 配置检查通过，但尚未替代真实设备验收。 |
| `N/A` | 当前 profile 或账号没有启用该能力。 |

## 统一前置条件

```bash
cd mobile
pnpm install --frozen-lockfile
pnpm run android -- --device
```

开发包使用 `APP_ENV=dev` 和独立的 `DEV_ANDROID_PACKAGE`。没有开发 Firebase 文件时，`DEV_GOOGLE_SERVICES_FILE` 留空；远程 FCM 不作为本地开发包的通过条件。切换过生产构建后，必须重新走 `pnpm run android`，不能直接用裸 `expo run:android` 绕过 clean prebuild。

至少准备：一个可登录账号、一个 MFA 账号或可触发 MFA 的账号、一个 Agent、一个已有长聊天、一个 Group（如果账号有权限）、一个可用插件连接、一个可用服务连接、一个自动化任务、一个待审批记录。Machines、OAuth、推送和真实语音需要额外环境，缺少时单独标 `BLOCKED`。

## 所有用例都要检查的横向规则

- Android Back 优先关闭键盘，再关闭内层 modal/sheet，再离开详情页，最后才退出根页面。
- 快速重复点击不会创建重复请求、重复消息、重复审批或叠加 modal。
- loading、空状态、错误、Retry、断网和恢复网络状态清楚，不能永久 loading。
- 切后台再回前台不会重复发送、重复 OAuth callback、重复创建语音连接或丢失表单草稿。
- status bar、navigation bar、键盘和底部操作栏不会遮挡内容；打开/关闭菜单后长列表滚动位置保持。
- 亮色/暗色/系统主题、触控目标、可访问性标签和 Android 平板/大屏布局没有明显回归。

## A. 冷启动、登录与公开入口（Agent 1）

| ID | 前置条件 | 步骤 | 预期结果 | 结果/证据 |
| --- | --- | --- | --- | --- |
| AUTH-01 | 干净安装 dev 包 | 冷启动；观察 splash、配置加载、Auth 页 | 无崩溃、无永久 loading；安装包名为 dev package；没有生产 Firebase mismatch | `TBD` |
| AUTH-02 | Auth 页可访问 | 检查邮箱/密码、动态启用的 Google/GitHub/Apple、Terms、Privacy | 登录入口与 iOS 保持一致；不会只剩邮箱；法律页可打开并能返回 | `TBD` |
| AUTH-03 | 错误凭据 | 空值、错误邮箱、错误密码、快速重复点击登录 | 有明确错误；按钮有防重复提交；输入状态合理保留 | `TBD` |
| AUTH-04 | MFA 账号 | 正确凭据→错误验证码→重试→正确验证码 | MFA 可恢复；失败不会留下半登录会话；成功只建立一个会话 | `TBD` |
| AUTH-05 | provider 已启用 | Google/GitHub/Apple 分别完成、取消、失败、回前台 | callback 只处理一次；取消不误报成功；失败回到可重试状态 | `TBD` |
| AUTH-06 | 已登录和未登录各一次 | 登录前/登录后打开 Terms、Privacy；切后台再回来 | 正确落在 Auth 或 Workspace sheet；底层聊天/输入状态不被错误重置 | `TBD` |
| AUTH-07 | 有效/无效/过期设备码（如启用） | 设备登录手动输入、扫码、取消、过期；验证 Back | preview、状态和错误正确；相机拒绝可重试；Back 先退出 scanner | `TBD` |
| AUTH-08 | Agent Key 能力已启用 | 输入/扫码 code；preview、approve/deny/expired；从 Quick action mint code | code 状态正确；secret 只展示一次；切后台/取消清理敏感值 | `TBD` |
| AUTH-09 | 已登录会话 | 模拟 401/refresh 失败或退出登录；杀进程重开 | 回到 Auth；旧账号 query/cache 清理；不能直接进入上一个账号 | `TBD` |

## B. Workspace 抽屉与 Assistant 全流程（Agent 2）

| ID | 前置条件 | 步骤 | 预期结果 | 结果/证据 |
| --- | --- | --- | --- | --- |
| CHAT-01 | 已登录 | 打开/关闭 drawer；点击 Chat、Agents & Groups、Plugins、Automations、Activity、Machines、Settings、Search、Recents、Scan QR、Terminal code、新建 Agent/Group | 每个入口只打开目标页面；Back/关闭优先处理 drawer 或 sheet；快速点击不叠加页面 | `TBD` |
| CHAT-02 | 有长聊天 | 滚动到中下部；点右上角 More/三个点；关闭 menu/sheet | 底层 chat 不跳顶部；关闭后滚动位置保留；复现 iPad 历史回归 | `TBD` |
| CHAT-03 | 新建或历史聊天 | 发送短文本、长文本、多行文本；键盘打开/收起；发送后重新进入 | 每条只发送一次；composer 正确清空；第一次 Back 只收键盘；不会跳顶或丢草稿 | `TBD` |
| CHAT-04 | 有文件权限 | DocumentPicker 选择、预览、删除、取消、上传失败/重试、发送附件 | 取消不会残留 pending；失败有明确状态；附件消息只发送一次 | `TBD` |
| CHAT-05 | 有 Agent/mention 数据 | Agent chooser/manage；@mention 插入/删除；切换 Agent；打开 history/details/title/delete modal | 路由和 modal 层级正确；切换 Agent 不串聊天；删除有确认；返回回到原聊天 | `TBD` |
| CHAT-06 | 有 stream/activity 卡片 | 发送触发 stream/tool activity 的消息；Stop、Retry、Refresh history | pending 到完成/失败正确；Stop 只执行一次；Retry 不复制消息 | `TBD` |
| CHAT-07 | 有 acknowledgement 卡片 | 打开 Allow、Deny、过期、recovery-required、authored skill review | 卡片和 Activity 状态一致；错误可重试；不会永久 loading | `TBD` |
| CHAT-08 | voice feature 和麦克风可用 | composer 开始、说话、停止、编辑识别文本、发送、取消；切后台/Back 中断 | 语音是 composer 入口；文字显示在 composer；没有第二个重复语音组件；中断可恢复普通输入 | `TBD` |
| CHAT-09 | 有历史聊天/多个 Agent | history、load more、latest、recents/search；冷启动 conversation deep link（如启用） | 正确打开目标 conversation；加载/空/错误状态清楚；返回栈不乱 | `TBD` |

## C. Agents、Groups、Plugins、Services、Machines（Agent 3）

| ID | 前置条件 | 步骤 | 预期结果 | 结果/证据 |
| --- | --- | --- | --- | --- |
| WORK-01 | Agent 列表数据 | 搜索、清空、过滤、刷新、空/错误/Retry、打开详情 | 列表状态准确；刷新不会重复请求；详情返回保留当前筛选 | `TBD` |
| WORK-02 | 有创建权限 | 创建 Agent；填写 name/description/model/permissions/services；保存、取消、dirty Back | 校验明确；不会重复创建；取消不保存；创建后进入正确详情 | `TBD` |
| WORK-03 | Agent 详情数据 | 编辑 profile/model/grants/memory/channels/threads/skills/automations；destroy/delete（按权限） | 权限限制正确；冲突可 reload；destroy/read-only/delete confirmation 正确 | `TBD` |
| WORK-04 | Group 权限和 Agent 数据 | Groups tab；创建/编辑 Group；选择成员/lead/参与者；保存、离开、删除 | dirty 状态和确认正确；成员/权限错误清楚；列表刷新一致 | `TBD` |
| WORK-05 | Group chat | recipient pills、mention、附件、发送、pending action、feedback、settings | 群聊和个人聊天不混淆；操作只执行一次；Back/键盘/modal 不丢草稿 | `TBD` |
| WORK-06 | Plugins 数据 | Connectors/Skills tabs；Added/Discover、搜索、刷新、connector manage | 空/错误/加载状态正确；连接状态不被误判；返回层级正确 | `TBD` |
| WORK-07 | Skills 数据 | detail、versions、history、preview、test、attach/toggle/detach、authored review | 版本和绑定状态准确；审批/拒绝/刷新正确；权限错误可理解 | `TBD` |
| WORK-08 | Services catalog/connection | Connected/Explore、搜索、API key/token exchange/platform/OAuth/Web handoff、回前台 | OAuth 取消不误报成功；credential 不泄露；连接状态以服务端为准 | `TBD` |
| WORK-09 | Service manage 数据 | enable/disable、provider permissions、proxy copy、replace credential、remove、history | 危险操作有确认；剪贴板/浏览器返回不泄露；history metadata 正确 | `TBD` |
| WORK-10 | Machine/node 已部署，否则 BLOCKED | Machines、MachineDetail、Policy、Activity/filter/detail、SavedLogin、AgentMachines、Desktop；在线/离线/Back | 有环境时逐页验证；离线/权限错误清楚；Desktop 关闭后恢复工作区 | `TBD` |

## D. Automations、Activity/Approvals、Account/Settings（Agent 4）

| ID | 前置条件 | 步骤 | 预期结果 | 结果/证据 |
| --- | --- | --- | --- | --- |
| OPS-01 | 自动化权限 | 列表搜索/status/agent filter；创建向导 Task→When→Delivery→Review；保存/取消 | 字段校验和 dirty Back 正确；不会重复创建；时区和间隔校验准确 | `TBD` |
| OPS-02 | 已有 automation | detail；pause/resume、Run now、edit、delete confirmation、run history/filter/detail | 状态和列表一致；危险操作确认；运行记录分页/空/等待/失败清晰 | `TBD` |
| OPS-03 | Activity 有 pending/history | Pending/History/Active grants；detail/evidence；Approve/Deny/revoke | 只处理一次；状态、toast、grant/revoke 与服务端一致；modal Back 正确 | `TBD` |
| OPS-04 | 推送/deep link 可用，否则 BLOCKED | 前台通知、后台/终止态点击、重复点击；`nyxid://challenge/<id>` | 打开正确 ActivityDetail；重复 tap 不重复处理；无 active route 有安全 fallback | `TBD` |
| OPS-05 | AccountSettings 可访问 | profile/sign-in method、Notifications、Telegram、Display/theme、Voice、Privacy、Usage/Credits、Terms/Privacy | 每个设置页能打开、返回和重新进入；主题/通知/Telegram 状态持久化正确 | `TBD` |
| OPS-06 | AccountSecurity 数据 | display/avatar；MFA setup/confirm/recovery codes/disable；sessions revoke | 敏感值不在后台保留；确认和错误清楚；撤销当前会话回 Auth | `TBD` |
| OPS-07 | 退出/删除能力 | Sign out confirm/cancel；杀进程重开；Delete account confirm/cancel/error（按环境） | 退出清理会话和缓存；取消不退出；删除失败不伪造成功 | `TBD` |

## E. Android 原生、权限、深链接和横向质量（Agent 5）

| ID | 前置条件 | 步骤 | 预期结果 | 结果/证据 |
| --- | --- | --- | --- | --- |
| NATIVE-01 | 当前分支和 Android SDK | dev/prod Expo config；串行 clean prebuild；检查 package、Firebase、intent filters | dev 不注入生产 Firebase；prod JSON package 精确匹配；配置可复现 | `TBD` |
| NATIVE-02 | Android 真机/模拟器 | camera/mic/notification 权限分别允许、拒绝、永久拒绝→系统设置→重试 | 文案和功能对应；拒绝后可恢复；无权限时核心聊天不崩溃 | `TBD` |
| NATIVE-03 | Android hardware Back | keyboard、drawer、More menu、sheet、详情、dirty editor、根页面逐层按 Back | 优先关闭当前层；不误退出、不丢草稿、不跳错误页面 | `TBD` |
| NATIVE-04 | offline/online | 在 Chat、Agents、Plugins、Services、Activity、Settings 断网→Retry→恢复 | OfflineBanner/错误/Retry 正确；恢复后刷新；缓存不伪装成空数据 | `TBD` |
| NATIVE-05 | background/foreground | 在 OAuth、voice、upload、stream、approval、editor 中切后台再回来 | 不重复 callback/请求/发送；敏感数据按规则清理；页面恢复 | `TBD` |
| NATIVE-06 | 深链接配置 | `adb shell am start -a android.intent.action.VIEW -d 'nyxid://challenge/<id>' <dev.package>`；设备登录 link；前台/后台/冷启动 | 已登录打开目标详情；未登录进入正确认证；非法/过期参数安全 fallback | `TBD` |
| NATIVE-07 | phone + tablet/large screen（如有） | 核心页面横竖屏/窗口变化；长列表、sheet、键盘、底部栏 | safe area、触控目标、文字和 modal 不重叠；iPad 菜单滚动回归不出现 | `TBD` |
| NATIVE-08 | release Java/SDK，否则 BLOCKED | `pnpm build:android` 或 CI release job；观察 Gradle 命令和失败日志 | signing 密码不在 argv/log；AAB 生成；CI 检查通过 | `TBD` |

## 明确不纳入当前 Android 通过条件的能力

需要 Web 控制台的完整 Channel/Trigger 管理、Studio 资源管理、OAuth scope 编辑、组织创建/邀请、shared grant cascade、尚未有安全 native admission 契约的特殊 provider SDK、credential node/SSH setup、依赖支付平台回跳契约的 checkout，不作为当前 Android 页面通过条件。缺少账号、机器节点、Firebase/APNs、OAuth provider 或服务器权限时，条件能力记录为 `BLOCKED` 或 `N/A`，不能用“页面打开了”代替完整集成通过。

## Agent 结果汇总

本轮验证为五个独立分支，全部从同一个固定基线 `0246eebd3d587fc4805db09371ba8d36dd5dee41` 创建。每个分支使用独立 worktree，未修改业务代码；这样每组结果都能单独复核，后续如果发现缺陷也可以在对应分支隔离修复。

| Agent | 分支 | Worktree |
| --- | --- | --- |
| Agent 1 | `codex/android-qa-auth` | `/Users/abigaildeng/.codex/worktrees/android-qa-auth/NyxID` |
| Agent 2 | `codex/android-qa-chat` | `/Users/abigaildeng/.codex/worktrees/android-qa-chat/NyxID` |
| Agent 3 | `codex/android-qa-workspace` | `/Users/abigaildeng/.codex/worktrees/android-qa-workspace/NyxID` |
| Agent 4 | `codex/android-qa-ops` | `/Users/abigaildeng/.codex/worktrees/android-qa-ops/NyxID` |
| Agent 5 | `codex/android-qa-native` | `/Users/abigaildeng/.codex/worktrees/android-qa-native/NyxID` |

| Agent | 覆盖范围 | 状态 | 报告/证据 |
| --- | --- | --- | --- |
| Agent 1 | Auth、MFA、Social OAuth、Device Login、Agent Key、公开入口 | `CODE-PASS / BLOCKED` | 独立分支聚焦测试 201 通过，扩展安全/session 选择集 240 通过，0 失败；真实 Android、provider、MFA backend、FCM 未可用，AUTH-01/05/07/08/09 的设备验收待补 |
| Agent 2 | Workspace、Assistant、Chat、附件、acknowledgement、语音 | `CODE-PASS / BLOCKED` | 独立分支聚焦测试 942 通过，0 失败；adb/emulator/Java 不可用，CHAT-02/03/04/08 的真实滚动、键盘、权限、麦克风、前后台待补 |
| Agent 3 | Agents、Groups、Plugins、Services、Machines | `CODE-PASS / BLOCKED` | 独立分支聚焦测试 329 通过，额外导航选择集 377 通过，0 失败；真实 backend、OAuth provider、机器节点和 Android 设备不可用，WORK-08/09/10 的集成验收待补 |
| Agent 4 | Automations、Activity/Approvals、Account、Settings | `CODE-PASS / BLOCKED` | 独立分支聚焦测试 166 通过，0 失败；真实账号、FCM、Telegram、MFA、删除账户和 Android 权限不可用，OPS-04/05/06/07 的手测待补 |
| Agent 5 | Android 原生、权限、深链接、离线/前后台、构建 | `CODE-PASS / BLOCKED` | 独立分支聚焦测试 65 通过，0 失败；dev/prod Expo config、Firebase 选择、scheme、intent filters、相机/录音权限和签名 helper 已静态核对，adb/emulator/Java 不可用，NATIVE-02~08 的真实设备或 release 构建待补 |

## 已完成的代码级证据

以下结果来自五个独立分支，只证明状态模型、导航模型、API contract、Expo 配置和组件逻辑；它们不能替代真实 Android 设备验收：

- Agent 1：认证、MFA、OAuth callback、Device Login、Agent Key、session persistence、法律页和导航测试 201 个通过，扩展选择集 240 个通过，0 失败。
- Agent 2：Assistant、composer、附件、stream、acknowledgement、voice、drawer、keyboard、sheet、recents/search/deep link 测试 942 个通过，0 失败。
- Agent 3：Agents、Groups、Plugins、Services、Machines、desktop route 和导航测试 329 个通过，额外导航选择集 377 个通过，0 失败。
- Agent 4：Automations、Activity/Approvals、Account、Security、Notifications、Display/Privacy、Sessions、MFA lifecycle 测试 166 个通过，0 失败。
- Agent 5：Firebase/profile config、signing helper、linking、orientation、auth/navigation 聚焦检查 65 个通过，0 失败；`node --check`、`git diff --check` 通过。带最小测试环境变量的 Expo config 检查确认 dev package `dev.nyxid.nativecheck` 不注入 Firebase，prod package `fun.chronoai.nyxid` 使用 `./google-services.json`。

## 当前真实设备阻塞

当前执行环境没有 `adb`、Android emulator、Java runtime 或真实 Android 设备。因此以下项目必须在 Android 真机/模拟器上补测后，才能把对应行从 `BLOCKED` 改成 `PASS`：麦克风/相机/通知权限、hardware Back、键盘和长列表滚动、iPad 历史 More 菜单回归、OAuth 浏览器回调、FCM 前后台/冷启动、Telegram/系统设置回跳、横竖屏/平板布局、release Gradle/AAB 构建。

原生静态检查还观察到自定义 scheme 与 Expo 生成的 scheme intent filter 可能在 manifest 中重复；当前不影响 intent resolution，但应在真机 deep-link 验收时确认不会出现错误路由。
