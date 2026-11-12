# Android 真实模拟器回归记录：2026-11-11

> 版本追加：同步团队 main 后的新 UI、原生 Debug 重建及 2026-11-12 验证见 [共享语音对齐记录](MOBILE_VOICE_PARITY_2026-11-12.md)。本页既有测试计数和截图保留为历史证据，未自动升级为最新版本全流程通过。

本报告记录用户完成 Google 登录后，主执行 agent 在 Android 模拟器中真实操作的结果。配套 [43 项聚合用例矩阵](ANDROID_FULL_REGRESSION_TEST_MATRIX.md) 保留 `PASS`、`PARTIAL`、`BLOCKED` 的差异；页面可打开、代码测试通过和整个用户路径通过是不同证据。

当前记录覆盖截图 001–249，并保留第 206、228 次新 UI dump 失败记录。已完成带音频权限的 APK 重装、在线冷启动保留登录，以及多条离线/恢复路径。最终无宿主音频的语音控制会话进入 Listening 并维持到 3:01 空闲结束，早先约 10 秒控制连接错误未再现；真实录音和播放仍未验证。离线冷启动黑屏未通过。已清理两个具名 QA 聊天及最后一次可明确识别的语音测试聊天，249 回到保留登录的 Home。

43 项聚合用例当前为 **1 `PASS`、37 `PARTIAL`、5 `BLOCKED`**。AUTH-01 的未登录公开入口、OPS-01 的实际应用并清空 Agent 筛选仍缺少完整证据，保留原有范围并标 `PARTIAL`。本报告不是全部流程的通过声明。

## 执行环境和证据位置

| 项目 | 本轮实际情况 |
| --- | --- |
| 工作分支 | `codex/android-build-fixes`；开始实测时 HEAD `d633f15f0430de6bf8e78dcb196a8923a3bceb2a`，随后在同一工作树修复移动端问题。 |
| 仓库/PR | 个人 fork `AbigailDeng/NyxID`，团队仓库 PR [#2348](https://github.com/ChronoAIProject/NyxID/pull/2348)。 |
| 设备 | `NyxID_QA_API34` / Pixel 7 / Android 14 API 34 ARM64 / `emulator-5554` / 1080 × 2400。 |
| 原生包 | `dev.nyxid.nativecheck`，versionName `1.0.4`，versionCode `1`；真实 debug APK 已构建安装，记录 `BUILD SUCCESSFUL`、464 tasks。 |
| SDK/JDK | SDK `/Users/abigaildeng/Library/Android/sdk`；Homebrew OpenJDK 17。旧报告“没有 adb/emulator/Java”不成立。 |
| 运行方式 | 工作树 `mobile/` 的 Metro + 真实后端；源码热更新和需要重新安装的原生 Manifest 修改分别记录。 |
| 身份/操作边界 | 用户本人完成 Google 登录；未登录 Apple。既有个人账户、连接、会话和自动化以只读/取消为主；写测试只使用明确的 QA 资源。 |
| UI 操作者 | 主执行 agent 独占模拟器。其他 agent 做代码分析、针对性修复、单测与独立审查，没有并发控制 UI。 |
| 本地证据 | `/tmp/nyxid-android-runtime/NNN-description.png` 及 `-small.png`。只在文档引用文件名，未提交截图、UI XML、OAuth URL、账号凭据或一次性 code。 |

未把 UI 动画中的过渡截图直接当作稳定布局缺陷。设备操作按新 UI dump 定位，避免混用原图像素与缩小图坐标。第 206、228 次 dump 都没有获得新的 root，不能引用磁盘残留的旧 XML 来证明当时页面、控件或操作结果；第 227 次离线黑屏保留截图证据，后续状态不从旧 XML 推断。后续完整结果仍应绑定最终提交和实际安装的 APK。

## 真实执行的用户路径

### 聊天、详情和输入

| 证据 | 操作与实际结果 | 尚未覆盖 |
| --- | --- | --- |
| `001-chat`、`003-chat-details-settled` 至 `012-history-search-empty` | 进入已有聊天；打开 Chat options/Details、Title editor 后返回、Working context、History/搜索无匹配；已有聊天删除仅打开确认并取消。 | 没有重置已有上下文、改名或删除已有聊天；历史分页未完整执行。 |
| `013-new-chat` 至 `016-chat-reply` | 在新 QA 聊天输入并发送短文本，真实 running 后得到回复。 | 快速重复提交、网络失败 Retry、所有私聊草稿切换组合未穷尽。 |
| `017-attachment-menu` 至 `023-file-picker-settled` | 打开 Camera/Photos/More/Files 入口和系统文件选择器；取消后返回聊天。 | 相机拍照/Photos 成功采集未完成。 |
| `124-file-picker` 至 `128-attachment-send-result`、`131-attachment-preview` | 将无敏感内容的 75-byte `nyxid-android-qa-20261111.txt` 放入 Downloads；选择、移除、重选、上传 ready 并发送成功。预览打开系统分享面板后 Back，未分享给外部应用。 | 未注入上传失败或验证重试；Crew 附件未测。 |
| `152-global-search`、`153-scroll-test-running` | 全局搜索打开 QA 聊天；发送用于长视口验证的编号内容。 | 所有搜索/Recents/deep link 冷启动组合未执行。 |
| `154-menu-scroll-baseline` 至 `157-menu-scroll-retained` | 底部原为 126–150，滚到 111–139 后打开 Chat options；返回后仍为 111–139。该实际路径通过，未跳到顶部。 | 大屏/横屏相同回归未测。 |
| `158-permission-menu-final`、`159-settings-popup-final` | composer 权限面板完整；Chat details → Permissions 弹出菜单默认完整展示 Full access 长说明、Auto 和 Ask every time 完整说明，不需先滚动。 | 系统大字和旋转仍只有聚焦逻辑测试，尚无设备验收。 |

### 语音和原生权限

| 证据 | 操作与实际结果 | 结论边界 |
| --- | --- | --- |
| `020-mic-permission`、`024-dictation` 至 `029-dictation-start`、`065-dictation-service-error` | 麦克风先拒绝后允许；该 AOSP 镜像没有 Android RecognitionService。修复后给出持续、可理解的识别不可用提示，关闭提示保留原草稿。 | 证明无识别服务时的恢复行为；没有真实语音转写，不计 dictation 成功。 |
| `067-live-voice` 至 `070-live-voice-retry` | 实际打开语音 composer 并启动；发现系统麦克风权限 Activity 引起 inactive 误判，以及错误提示被自动追底裁掉。 | 保留为缺陷发现证据，不能当作通话通过。 |
| `140-voice-after-permission-fix`、`141-voice-connection-result` | 权限处理修复后进入 Connecting，不再立即报 inactive；错误提示完整可读，没有自动滚到底部只露半行。179 实际 Home/回前台已确认通话停止；180 关闭语音后键盘输入正常。 | 媒体仍报错；日志明确指出原生包缺少 `MODIFY_AUDIO_SETTINGS`。已实际重建安装并确认两项权限 granted；进一步跨过麦克风初始化的焦点修复和控制连接失败见后续记录。 |
| `215-final-voice-panel`、`216-final-control-result`、`217-final-voice-steady` | Start 后进入 Listening；216 为 0:08，217 为 1:18，没有复现 178 约 10 秒断开。 | 证明该次会话启动并维持控制连接；本轮为 `-no-audio`，不能据此证明录音或播放。 |
| `218-voice-microphone-muted`、`219-voice-options-or-result` | 2:47 时麦克风静音，并收到服务端字幕形式的空闲提醒；3:01 正常因 quiet call 结束。 | 区分正常空闲结束和连接错误。服务端空闲提醒不是用户语音识别结果，仍未证明真实转写或双向声音。 |
| `142-camera-permission` 至 `148-code-back-stack` | Scan QR primer → 系统相机拒绝 → 重试允许；手动设备码输入、无效码校验、Back 退出 scanner。 | 未用真实有效/过期请求完成扫码与审批，不代表 auth-device 审批全链通过。 |

### Settings 和账户只读路径

| 证据 | 操作与实际结果 | 未执行的写操作/集成 |
| --- | --- | --- |
| `030-settings` 至 `037-listening-menu` | Settings、Display、composer 高度预览、Voice、语言与 listening mode 选择器可打开返回；Light → Dark → Automatic 恢复。语言/语音模式选择器取消返回。 | 语言/语音模式的全部保存、重启持久化以及系统字体/窗口变化未完整测试。 |
| `038-notifications` 至 `041-approval-settings-menu` | Notifications 实际完成加载；Telegram link 打开后取消；approval policy 菜单打开但没有修改既有策略。 | 未绑定 Telegram、未发布 code、未验证远程推送。初始加载成功不能证明所有失败/Retry 分支。 |
| `042-account-security` 至 `048-security-bottom` | Name/avatar editor、Security、sessions、授权 app/broker 的只读路径已查看并返回。 | 未更改个人资料、MFA、recovery codes，也未 revoke 现有会话或 broker。 |
| `049-usage` 至 `052-privacy-policy` | 查看余额/allowances、Privacy、Terms、Privacy Policy，返回正常。 | 未做支付、余额变更或登录前法律页组合。 |
| `053-settings-dangerous-actions` 至 `055-delete-account-confirm` | Sign out 确认选 Cancel；Delete account 选 Keep my account。 | 没有真的退出或删除账户，当前认证会话保留。 |

### Automations、Activity 和 Machines

| 证据 | 操作与实际结果 | 结论边界 |
| --- | --- | --- |
| `056-automations` 至 `060-automation-agent-filter` | 自动化列表、搜索无匹配后清空、Active 空态后回 All、Agent picker。 | 符合当前只读原生界面；不以不存在的创建向导或 Run now 控件作为验收目标。 |
| `061-automation-detail` 至 `064-ask-automation-draft` | 详情、运行历史、单次运行详情；Ask NyxBot 打开预填聊天草稿。 | 没有运行、暂停、修改或删除既有自动化；分页和所有状态未穷尽。 |
| `071-activity`、`072-active-grants` | Activity refresh 后 Pending/History 为空；Active grants 为空。 | 没有安全待审批数据，因此未作 Approve/Deny/revoke，也未验证审批卡片或推送点击。 |
| `073-machines`、`074-saved-logins`、`090-agent-machine-access` | Machines、Saved logins、Agent machine access 正确显示空态。 | 账号没有可用测试机器/保存登录，MachineDetail/Policy/Desktop 的深层集成路径阻塞。 |
| `149-terminal-code` 至 `151-terminal-key-list` | 打开 Terminal login code 入口及 Agent Key 选择，Back 返回。 | 未 mint/redeem code，未将终端登录或 Agent Key 审批记为通过。 |

### Plugins/Connectors、Agents 和 Crews

| 证据 | 操作与实际结果 | 结论边界 |
| --- | --- | --- |
| `075-plugins` 至 `084-skill-agent-picker` | Connectors 搜索/清空，GitHub 管理、scopes 展开、history、advanced 只读；Skills 详情、版本 picker Back、Agent picker Cancel。 | 未替换或删除现有凭据，没有完整 provider OAuth；没有给现有 Agent 改 pin/remove 技能。 |
| `222-skills-offline-no-cache` 至 `226-skill-detail-offline` | 222 离线无缓存时分别提示 Added/catalog unavailable offline；223 复网自动取数，224 目录实际加载；225 再断网仍可读缓存目录，226 该次离线点击未进入详情；阻止远端调用另由聚焦测试验证。 | 覆盖 Skills 的无缓存、重连和缓存路径；不替代精确 pin/remove、版本冲突或全部离线详情组合。 |
| `085-new-agent` 至 `096-agent-filter-menu` | 创建临时 QA Agent；dirty Back 选 Keep editing；保存后进入详情、改名 Save；机器空态、Agent 自动化筛选；Agent chooser、Manage Agents、搜索和 filter。 | 未穷尽无效字段、重复提交、丢弃新表单等分支。 |
| `098-agent-permissions` 至 `106-memory-forget-confirm`、`122-permission-filter-fixed`、`123-grant-removed` | 内部 NyxBot Voice 错误出现在可授权服务列表，保存得到真实 ValidationError；修复后列表 38 → 37，内部 voice 服务消失。GitHub grant 添加 Save，随后移除 Save。QA Memory 添加后确认忘记并移除。 | 只操作临时 QA Agent，没有修改用户既有 Agent 的连接权限或记忆。 |
| `107-crews` 至 `120-qa-crew-deleted` | 创建仅含 QA Agent 的临时 Crew；多行输入、真实发送并回复；改名持久化；mention picker 插入；最后确认删除 Crew。 | 没有完整验证所有成员/lead/Leave/权限错误组合，也未验证 Crew 附件和 pending actions。 |
| `133-agent-destroy` 至 `138-qa-agent-deleted` | 对临时 QA Agent 执行 Destroy，确认只读状态，再 Delete permanently 并确认移除。 | 没有销毁真实业务 Agent；临时 QA 聊天的最终清理范围见末尾记录。 |

## 本轮发现并修复的问题

以下记录区分源码修复、聚焦验证和设备复验；即使实现已改，也不自动等于所有相关集成路径通过。

| # | 缺陷与修复 | 当前证据/状态 |
| --- | --- | --- |
| 1 | Auth 的公开配置失败同时写入 query error 和 loginError，配置 Retry 恢复后可能仍显示过期错误。配置错误由 query 统一负责，保留真实 OAuth/browser 错误。 | 聚焦回归已验证超时 → 重试 → provider 恢复；当前登录会话未退出，最终包完整登录复验仍待补。 |
| 2 | SettingsList 权限菜单用固定估算总高，长说明裁掉末项；仅放宽 maxHeight 仍受原生隐式测量影响。改用真实内容高度显式设置 surface height，受可用窗口限制；保留向上锚定、滚动、尺寸变化关闭和异步 measurement generation。 | 009、129 保留失败过程；159 最终默认完整可见。短菜单、后续换行、旋转/迟到回调聚焦回归通过；真实大字/横屏仍待测。 |
| 3 | Android Display/Voice 说明仍写 “iPhone”。改为设备中性文案。 | 源码/针对性检查已完成；相应页面实测可达。 |
| 4 | Android 无 RecognitionService 时 dictation 启动不可用、错误恢复不足。加入预检、持久可关闭提示和 8 秒启动超时；保留草稿。 | 065 实际不可用场景验证；该镜像缺少识别服务，因此实际识别依旧 `BLOCKED`。 |
| 5 | Voice composer 所有内容变化都 scrollToEnd，启动错误只露下半行。可见错误归顶，只有 active 且有 captions 时跟随底部；隐藏 settings/options 错误不打断字幕；等高错误替换也主动归顶。 | 141 完整错误可读；组件与聊天语音入口聚焦回归通过。没有扩大 composer 外观或将媒体失败冒充通话成功。 |
| 6 | Android 麦克风系统权限 Activity 被误判为离开应用，启动立即 inactive。已有权限不重复请求；限定 60 秒权限请求、5 秒前台 handoff，真实离开仍停止。 | 140–141 修复后进入 Connecting，未再因权限窗口误停；真实 Home 停止已检查。最终原生包已安装；179/180 覆盖真实后台终止及恢复输入。215–219 的控制会话持续至正常空闲结束；双向音频仍未验证。 |
| 7 | AgentPermission 将内部 `nyxbot-voice` canonical/legacy 连接当成可授权工具；真实保存失败。从可授权列表精确排除内部连接，不扩大到不相关服务。 | 100 为失败；122 列表排除，101/123 合法 GitHub grant 添加和移除保存成功。 |
| 8 | 原生 Manifest 缺少 `MODIFY_AUDIO_SETTINGS`，Chromium WebView 不能暴露录音设备。加入 Expo Android permissions 并增加配置回归。 | 日志已定位缺失；修复需要 native rebuild/reinstall，JavaScript HMR 不能代替。ARM64 debug 重建安装成功，dumpsys 确认权限 granted。178 控制错误保留为历史失败；最终 215–219 未再现该错误，但 `-no-audio` 不能证明真实录音和播放。 |
| 9 | Android WebView 未获得文档焦点，媒体 prepare 停在 microphone/pending。仅显式 prepare 前调用官方 `WebView.requestFocus()`，随后发送原准备命令。 | 177–178 focus=true 并越过麦克风初始化；iOS 和其余控制不获取焦点。215–219 验证最终控制连接及正常空闲结束，仍保留声音验证缺项。 |
| 10 | Connectors 离线无缓存时把 pending/paused 当成无限 loading。按联网状态显示无缓存指引、保留缓存并阻止离线刷新。 | 199 无缓存重连提示，201 复网自动加载 32 条 Added，202 离线缓存可读。 |
| 11 | Skills 的 agent/Added/catalog/版本/预览/历史同样存在离线 pending，读取与修改入口缺少联网边界。加入各层离线指引、缓存展示、刷新和修改 guard，包括异步预检中断网后的写入拦截。 | 222–226 完成无缓存、自动重连、缓存可读的设备复验。对应 8 个新增回归及直接契约测试通过；完整 pin/remove 实际写路径仍未操作。 |

原有旧生产 APK 抢占自定义 scheme、模拟器物理键盘和 SDK PATH 发现等环境问题也已处理，但不把它们算成移动端业务代码修复。旧生产包从该测试模拟器移除后，当前 `nyxid://auth/social/callback` 只由 dev 包接收。

## 聚焦验证记录

历史五 agent 的 201/942/329/166/65 等结果属于先前固定基线的代码检查，完整保留在矩阵；扩展集合存在包含关系，不能相加。它们没有执行本轮设备 UI。

最终聚焦选择集显式执行 23 个直接测试文件，包含当时 diff 中全部 13 个 changed/new 测试文件，结果 **365/365 通过，0 失败、0 跳过**。没有运行全量移动端测试。精确命令如下，从本轮工作树根目录执行：

```bash
python3 ~/.codex/skills/frontend-incremental-pr/scripts/frontend_change_scope.py --repo . --base HEAD

cd mobile
pnpm exec tsx --test \
  src/components/settings/SettingsList.test.ts \
  src/features/auth/AuthHomeScreen.test.ts \
  src/features/auth/publicAuthConfig.test.ts \
  src/features/assistant/useComposerDictation.test.ts \
  src/features/assistant/useAssistant.dictation.test.ts \
  src/features/assistant/voice/useNativeVoice.test.ts \
  src/features/assistant/voice/AssistantVoiceConversation.test.ts \
  src/features/agents/agentContracts.test.ts \
  src/features/agents/AgentPermissionsField.test.ts \
  src/app/nativePermissions.test.ts \
  src/features/account/DisplayPrivacySettingsScreen.test.ts \
  src/features/account/voiceSettingsModel.test.ts \
  src/features/assistant/voice/dictationLocale.test.ts \
  src/features/assistant/AssistantScreen.voice.test.ts \
  src/features/assistant/ComposerDictation.test.ts \
  src/features/display/ChatDisplaySettings.test.ts \
  scripts/android-build-command.test.js \
  src/features/assistant/voice/voiceMediaDocument.test.ts \
  src/features/plugins/PluginsScreen.test.ts \
  src/features/plugins/pluginConnectors.test.ts \
  src/features/plugins/PluginSkills.test.ts \
  src/features/plugins/SkillLibrary.test.ts \
  src/features/plugins/pluginSkillsTypes.test.ts
# 23 文件，365/365 通过。
```

控制层临时诊断日志清理后，另执行下列不重叠的 2 文件集合，**65/65 通过**。已对照前述 23 文件 manifest，两个文件均不在前一个集合内，因此这两个最终集合共 **430 项不重复的代码测试通过**；其他历史选择集不再累加：

```bash
# 仍在 mobile/ 下。
pnpm exec tsx --test \
  src/features/assistant/voice/AssistantVoiceClient.test.ts \
  src/features/assistant/voice/voiceSocket.test.ts
# 2 文件，65/65 通过。
```

交付前另外按整个 PR 的共同基线 `3e430b8d42e422e2246dced6bfed5b865853f90d` 运行 scope analyzer，确认先前构建脚本提交中的 `src/lib/loadEnv.test.ts` 也属于 PR 改动测试。补跑该直接文件 **22/22 通过**，它与前 25 个文件无重叠，因此最终共 **26 个文件、452 项不重复代码测试通过**。三个先前改动的 JavaScript 构建文件语法也分别通过；本轮没有运行完整 suite：

```bash
# 工作树根目录。
python3 ~/.codex/skills/frontend-incremental-pr/scripts/frontend_change_scope.py --repo . --base 3e430b8d42e422e2246dced6bfed5b865853f90d
node --check mobile/scripts/lib/load-env.js
node --check mobile/scripts/android-build-command.js
node --check mobile/scripts/build-android.js
cd mobile
pnpm exec tsx --test src/lib/loadEnv.test.ts
```

23 文件批次还完成 28 个 changed TS/TSX 文件的 `transpileModule` 语法检查，0 diagnostics，以及 `git diff --check`。28 文件验证前后 SHA-256 一致，没有并发源码变更；`AssistantVoiceClient.ts` 在该语法批次中明确排除，后续控制层集合单列记录。语法检查不等于类型检查：没有可靠的 repository-native affected mobile typecheck，因此跳过本地 typecheck；完整前端套件、类型检查、lint 和生产构建由 GitHub CI 负责。

该次静态检查实际命令如下，从工作树根目录运行。脚本和 manifest 位于本机临时证据目录，需要连同日志一起查看，不能将这些临时产物当作已上传的 CI 附件：

```bash
node /tmp/nyxid-android-runtime/final-validation-syntax.cjs
git diff --check
```

完整可复核文件均在 `/tmp/nyxid-android-runtime/`：`final-validation-commands.sh` / `.json` 保存原始精确命令和工作目录；`final-validation-manifest.json` 保存 23 测试文件、全部 13 个 changed/new 测试、28 个语法检查文件及前后哈希；`final-validation-scope.json` 保存 analyzer 输出；`final-validation-tests.log`、`final-validation-ts-syntax.log`、`final-validation-diff-check.log` 保存实际输出；`final-validation-summary.md` / `.json` 保存结果汇总。

历史 Auth 54/54、菜单 14/14、语音入口 76/76 和共享菜单消费者 41/41 等检查保留为过程证据。这些选择集与最终批次存在包含或重叠，不额外加入最终 452。独立 reviewer 对 SettingsList、语音状态/滚动、VoiceMediaBridge、连接器和 Skills 的相关 diff 做过无修改审查，并按各自范围复跑聚焦检查；这些审查不替代设备录音、第三方认证或发布验证。

## 仍需执行或明确阻塞的路径

| 项目 | 原因与下一步 |
| --- | --- |
| 实时语音媒体/双向声音 | 已重建安装并修复 Android WebView 未获焦点导致的 getUserMedia 等待。178 的约 10 秒控制错误保留为历史失败；最终 215–219 在 `-no-audio` 下 Listening 维持至 3:01 正常空闲结束，未再现该错误。开启宿主音频曾两次导致 QEMU 主循环挂死，真实录音、播放和用户语音转写仍未验证；服务端空闲提醒字幕不能作为识别成功证据。 |
| Dictation 实际识别 | 当前 AOSP 镜像没有 RecognitionService；需具备识别服务的 Android 镜像/真机。无服务提示通过不等于识别成功。 |
| 离线/恢复、最终冷重启 | 183–202 已覆盖聊天草稿、Activity、Settings、Home、Connectors 冷数据及缓存、重连发送成功；222–226 补充 Skills 无缓存、自动重连和缓存目录；196–197 在线 force-stop 重开恢复原 Google 登录。227 离线冷启动黑屏，228 新 root dump 失败，因此该分支仍未通过。debug 包依赖 Metro，现有证据不足以归因为业务层故障；206/228 不能引用旧 XML。所有页面的异常组合仍未完整执行。 |
| MFA、失败登录、真正退出/401 | 尚未准备隔离挑战或释放当前会话；不能从用户 Google 成功推导这些路径。 |
| FCM/challenge 通知、审批卡片、Agent Key/device 有效/过期请求 | 缺少 dev FCM 配置和安全专用记录；本轮只有空态、入口和无效输入。 |
| Machines/Desktop、Saved login | 当前账号没有可用测试机器/保存登录；仅 inventory/权限空态被执行。 |
| Provider OAuth、凭据替换/撤销、Telegram | 为避免影响真实连接，只验证只读/取消路径；需可清理的测试连接与第三方配置。 |
| 横屏/平板/大字、全部错误/重复点击组合 | 尚未完成真实设备覆盖。窗口尺寸逻辑单测不能替代这些运行结果。 |
| Release AAB/signing/distribution | 当前是 debug APK；必须由对应 CI/发布环境单独验证。 |
| 临时 QA 数据清理 | QA Crew、Agent、Memory 和测试 grant 已清理。238–244 确认删除两个具名 QA 聊天；245–247 根据 3:01 空闲通话内容识别并删除最后一次语音 QA 聊天；Downloads 的 75-byte QA 文件已移除。更早的同名 New conversation 未逐一确认身份，保留，未批量删除。 |

## 最终 APK 和横向复验追加记录

- 新音频权限的第一次增量构建缺少 profile 环境，随后全 ABI 构建耗尽宿主磁盘；只清理本工作树的可再生成原生中间产物，未删除账户/AVD 用户数据。改为模拟器所需 ARM64 后构建成功：`./gradlew :app:assembleDebug -PreactNativeArchitectures=arm64-v8a --offline --no-daemon`，21 秒，464 tasks（35 executed/429 up-to-date）。JDK17、SDK 和无敏感信息的 dev profile 均以环境传入。此项是实际运行所需 debug 构建，未执行生产构建。
- `adb install -r` 成功；已安装包 `dumpsys package` 同时声明并授予 `RECORD_AUDIO` 和 `MODIFY_AUDIO_SETTINGS`，versionName 1.0.4。161 重启仍恢复登录。
- 164–170：原始“缺少 MODIFY_AUDIO_SETTINGS”日志消失，但 prepare 仍在20秒超时。临时严格白名单诊断确认 secure context、非 opaque、文档 visible，停在 `microphone/pending`；只读 CDP 发现 `document.hasFocus() === false`。
- 新增修复 #9：仅 Android 显式 prepare 前调用正式支持的 `WebView.requestFocus()`，随后发送原来的准备命令；iOS 和其他控制不获取焦点。177–178 文档实际 focus=true，越过麦克风初始化并创建会话，随后约10秒控制连接丢失。此时未证明实时语音可用。
- 179：连接期间按 Home，再回应用，通话停止且显示 inactive 的恢复说明。180：关闭语音后软键盘弹出、硬件输入成功，无焦点残留。
- 181–182：只对新创建的 QA 语音聊天改名为 `Android QA Voice 20261111`，保存后详情回显正确。
- 183：断网后发送被禁止，明确离线提示，草稿仍在输入框。187 Activity 和188 Settings 都有离线状态；190 Home 可读缓存 Agent。193 返回原聊天仍保留完整草稿；194–195 复网后只发送一次，得到真实 OK 回复。
- 196–197：force-stop 后冷启动恢复原 Google 登录。198：无有效事务的 access_denied auth callback 不改变既有登录状态，不代表所有 deep link 组合通过。
- 新增修复 #10：185–186 Plugins → Connectors 离线无缓存时长期 busy 骨架屏。修复 pending/paused 显示与网络 guard 后，199 无缓存时明确提示重连；201 恢复网络自动加载32条 Added；202 再次离线保留这32条缓存和离线提示。
- 新增修复 #11：203 Skills 同样离线持续显示 Loading added skills / Searching skill catalog。对 agent、skills、catalog、版本、包预览、历史的读取和修改入口分别加联网边界，无缓存提示重连，有缓存可读。包含异步预检过程中断网阻止后续写入。222 无缓存时分别出现 Added/catalog unavailable offline，223 复网自动取数，224 目录实际加载；225 再断网缓存目录可读，226 该次离线点击没有进入详情；阻止远端调用的边界由聚焦测试另行验证。
- 204–205：Display 文案实际显示 device，Voice 显示 System language 和 device's speech recognition。
- 215–219：实际从 Start 进入 Listening：0:08 → 1:18 → 2:47，静音控制可操作并收到服务端空闲提醒字幕，3:01 正常因 quiet call 结束。临时诊断记录控制握手成功，该次约三分钟控制连接没有复现 178 的十秒错误；取证后诊断已完整清理，`AssistantVoiceClient.ts` 对开始实测的 HEAD 没有最终 diff。最终模拟器为 `-no-audio`，这一结果只支持控制会话、静音状态和服务端提醒路径，不支持真实录音、播放或用户转写通过。
- 227 离线冷启动截图为黑屏；228 取新 root dump 失败。该分支保持未通过。Metro 是 debug 包的运行依赖，现有证据不能区分开发服务连通性与业务层问题；第 206、228 次均不得用已有旧 XML 替代当时失败的 dump。
- 229 恢复联网后进入已登录 Home，236 再次打开应用也恢复 Home。该阶段模拟器报告 EGL context restore 错误且绘图迟缓；AndroidRuntime 中的 FATAL 属于 UiAutomationService 重复注册，未发现该条日志对应应用进程崩溃。上述事实不能反推离线冷启动通过或断言黑屏根因。
- 238–241：对 `Android QA Voice 20261111` 进入详情、确认删除，结束到 New chat；242–243 同样删除 `Android QA 20261111`，244 Recent chats 中两个具名 QA 项均不再出现。245 根据 23:06 的空闲提醒及 3:01 receipt 确认最后一次语音 QA 聊天身份，246–247 删除并回到 New chat。未批量删除更早同名聊天。Downloads 中 QA 文本文件已移除；248–249 回到已登录 Home，并关闭临时诊断遗留的开发警告 toast。网络已恢复为 airplane off、Wi-Fi connected/validated，Google 会话保留。
- 带宿主音频启动的模拟器在实际 Start voice 后两次报 QEMU main loop/CPU thread hang 并退出；磁盘恢复后仍可复现。另有初次宿主磁盘耗尽和一次 crash-consent 阻塞，已分别处理，不能混成应用崩溃。最终取证必须区分宿主、媒体初始化、控制 socket 及 provider；无声音模式不记双向语音通过。

没有完成的子步骤继续保留 `PARTIAL` / `BLOCKED`，失败链路也不因源码测试通过而更改为 `PASS`。

## 本轮质量门禁结论

**BLOCKED（完整 Android 用户路径验收）**。已确认的移动端修复完成聚焦测试和独立代码审查，实际执行及修复记录可复核；真实录音/播放和识别仍无通过证据，离线冷启动仍未通过，表中外部集成和安全专用数据缺项也没有被替代验收。后续需要具备音频与识别服务的 Android 环境，以及 FCM、MFA、审批和机器的专用测试条件；不能据此报告全流程通过。
