# 移动端共享语音界面对齐验证：2026-11-12

## 版本与不一致的原因

Android 测试分支原先基于旧 main，仍使用 2026-11-05 的语音组件。团队 PR [#2322](https://github.com/ChronoAIProject/NyxID/pull/2322) 已合入新版移动端语音、iPad 适配和多语言（合并提交 `ef3f50d6936f58d7dc25ea004f32d1633b524efc`），但 Android 分支尚未同步；截图中的欢迎词、费用长段落和旧操作区来自这个版本差。

本次将团队 `upstream/main` 的 `24895be00bb0c09877192ea1b66092dc3efde8b3` 合并到个人 fork 的 `codex/android-build-fixes`。相对该 main 的业务改动限定在 `mobile/`，没有为 Android 另写一套语音 UI。iPhone、iPad、Android 复用 `AssistantScreen`、`AssistantVoiceConversation` 和 `VoiceComposerShell`；iPad 按窗口宽度调整布局。底层媒体仍按平台采用 iOS native WebRTC / Android WebView，不能把 UI 共用等同于相同原生媒体实现。

保留 Android 已修复的音频权限、WebView prepare 焦点、登录错误归属、profile/Firebase/Gradle signing、权限菜单测量、离线 Connectors/Skills 等行为；最新的 iOS readiness、recovery、bounded drain、Crew dictation 和 audio ownership 同时保留。新增用户提示复用 main 的 `src/i18n`，简体/繁体 JSON 各新增 41 个翻译键，英文使用现有 key/fallback 约定，没有引入第二套多语言框架。

## 新增回归与审查修复

- 权限菜单将异步内容尺寸回调绑定到当前菜单代次，关闭/旋转/重开后的旧回调不能改变新菜单高度。
- Android 首次 dictation 权限请求允许受限的 Activity 后台转换；真正后台录音仍取消，权限和前台 handoff 有各自期限，取消后的晚到授权不能重启录音。
- `useTranscriptScrollFollow` 的 reset effect 跟踪 scope generation，修复已提交 A → 弃置 B render → 再提交 A 时旧 deadline 失效后永远遮住聊天的源码竞态。该竞态已通过执行真实 hook 源码的回归复现；不能据此断言它就是此前某次模拟器 skeleton 的唯一运行根因。
- 临时诊断日志已删除。独立 reviewer 检查最终 shared UI、媒体分流、Android 保护、多语言及上述竞态，没有剩余确认代码问题。

## Android 新包与实测证据

这部分是同步 main 后的新运行证据，与 [11 月 11 日报告](ANDROID_RUNTIME_REGRESSION_2026-11-11.md) 的 001–249 截图和 452 项测试分开记录。旧的 43 项聚合矩阵仍保留 1 PASS / 37 PARTIAL / 5 BLOCKED，未因此次局部验证改为全通过。

- AVD：`NyxID_QA_API34`，Pixel 7 / Android 14 API 34 ARM64，`emulator-5554`。
- 以 `-allow-host-audio` 启动，未用 `-no-audio`。Google 登录保留；没有登录 Apple，没有清空用户应用数据。
- 旧 APK 在加载上游新增依赖时报告缺少 `ExpoSpeech`。已执行必要的 ARM64 native Debug 重建，483 tasks（74 executed / 409 up-to-date），1m53s，`adb install -r` 成功。
- Metro 8081 监听 LAN，adb reverse 8081 已建立；debug 应用依赖这个开发服务。

| 本机证据（`/tmp/nyxid-android-runtime/`） | 实际观察 | 边界 |
| --- | --- | --- |
| `257-unified-native-home-small.png`、`270-parity-final-home-small.png` | 安装新壳后恢复已登录 Home。 | 不代表全新 OAuth 或 MFA 已复验。 |
| `260-unified-active-voice.png`、`261-unified-compact-voice.png` | 首次新 UI 运行到 Listening，观察到主聊天遮罩残留。 | 保留失败证据，不用后续正常运行覆盖它。 |
| `266-parity-call.png`、`268-parity-muted.png` | 冷启动后 Listening，聊天区无遮罩；静音变为 Muted，按钮呈红色禁用麦克风图标。 | 静音状态变化不是声学静音的独立测量。 |
| `271-final-voice-small.png` → `272-final-compact-small.png` → `274-final-expanded-settled-small.png` | 通话展开 → 点击聊天区收起成底部一行 → 点击紧凑条重新展开；计时继续到 2:16，聊天未跳回 skeleton。 | 属于 UI 和控制会话验证；未进行用户说话/录音回放。 |
| `275-final-return-text-small.png`、`277-return-check-small.png` | 结束后服务端 receipt 为 2:42，显示通话结束；开发期源码热更新期间出现 retained finishing 恢复面板，Return to chat 未立即关闭。 | 不把这条热更新中的结束路径记为通过；最终冷启动结束路径另记。 |
| `279-cold-call-before-end-small.png` → `280-cold-call-ended-small.png` | 停止源码热更新后冷启动，重新进入 Listening，点击结束；1:33 receipt 显示通话结束，语音面板关闭、正常文字输入框恢复，聊天无遮挡。 | 本次结束/回文字路径通过；不替代真实声学输入/播放。 |

原生 debug 构建实际命令在 `mobile/android/`，以 dev profile、SDK/JDK 17 环境执行：

```bash
/Users/abigaildeng/.local/share/nyxid-android-qa/gradle-9.0.0/bin/gradle \
  :app:assembleDebug -PreactNativeArchitectures=arm64-v8a --no-daemon
```

## iOS 原生重建与设备对照

以同一份源码生成 iPhone/iPad 通用 arm64 Simulator Debug 包：`dev.nyxid.ipadqa`，1.0.4/build46，`UIDeviceFamily=[1,2]`。在被忽略的 `mobile/build/ios-ipadqa-runtime/` 副本安装 Pods（124 dependencies / 126 pods），没有修改 tracked iOS 原生工程。旧包缺少 ExpoSpeech 的加载问题消失。

最初 unsigned Debug 包缺少模拟器 Keychain entitlement；改为 Xcode ad-hoc simulator signing，最终 `NyxIDMobile.app-Simulated.xcent` 与缓存旧 build45 **完全一致**，`codesign --verify --deep --strict` 通过。覆盖安装后 iPad 原有 Google 会话恢复，未读取、复制或导出登录凭据。

实际原生 Debug 命令在隔离副本 `mobile/build/ios-ipadqa-runtime/mobile/ios/`：

```bash
xcodebuild -jobs 4 \
  -workspace NyxIDMobile.xcworkspace -scheme NyxIDMobile \
  -configuration Debug -sdk iphonesimulator \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath ../../DerivedData \
  CODE_SIGNING_ALLOWED=YES CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY=- \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES build
# BUILD SUCCEEDED
```

| 设备 | 当前实际结果 | 证据/限制 |
| --- | --- | --- |
| iPad Pro 13-inch (M5), iOS 26.3 | 覆盖安装后恢复已登录 Home，打开新版 shared voice，达到 Listening，布局与 Android 使用相同标题/模式/波形/麦克风/结束控件；结束后 receipt 1:14 且文字输入恢复；旧欢迎和费用长文不再出现。 | `281-ipad-signed-startup.png`、`282-ipad-shared-voice.png`、`284-ipad-ended-text.png`；主聊天无遮罩。此项未测试真人语音输入、ASR 或播放。 |
| iPhone 17 Pro, iOS 26.3 | 新包连接同一 Metro，加载登录页；Google/GitHub/Apple/email 均可见，没有缺少 ExpoSpeech 或 Keychain entitlement 错误。 | `283-iphone-shared-auth.png`；设备没有本轮已登录会话，未执行登录后的语音流程，没有登录 Apple。 |

截图同样仅保存在本机证据目录。iPhone 的启动验证不能写成已完成语音 UI/媒体验收；三端共用源码与两端实际语音控件对照分别记录。

## 当前版本的聚焦验证

34 个显式选择的依赖相关测试文件，**675 passed / 0 failed / 0 skipped**。包含相对 main 的全部 changed/new 测试；没有运行全量 suite。从 `mobile/` 执行：

```bash
pnpm exec tsx --test scripts/android-build-command.test.js \
  src/app/nativePermissions.test.ts \
  src/components/settings/SettingsList.android.test.ts \
  src/features/account/AccountSettingsScreen.test.ts \
  src/features/account/DisplayPrivacySettingsScreen.test.ts \
  src/features/account/LanguageSettingsScreen.test.ts \
  src/features/account/MobileSettingsMessages.test.ts \
  src/features/account/NotificationSettingsScreen.test.ts \
  src/features/account/VoiceSettingsScreen.test.ts \
  src/features/account/voiceSettingsModel.test.ts \
  src/features/agents/AgentPermissionsField.test.ts \
  src/features/agents/agentContracts.test.ts \
  src/features/assistant/AssistantScreen.voice.test.ts \
  src/features/assistant/ComposerDictation.test.ts \
  src/features/assistant/TranscriptViewport.test.ts \
  src/features/assistant/useAssistant.dictation.test.ts \
  src/features/assistant/useComposerDictation.test.ts \
  src/features/assistant/useTranscriptScrollFollow.test.ts \
  src/features/assistant/voice/AssistantVoiceConversation.test.ts \
  src/features/assistant/voice/VoiceMediaBridge.test.ts \
  src/features/assistant/voice/dictationLocale.test.ts \
  src/features/assistant/voice/useNativeVoice.readiness.test.ts \
  src/features/assistant/voice/useNativeVoice.test.ts \
  src/features/assistant/voice/voiceMediaDocument.test.ts \
  src/features/auth/AuthHomeScreen.test.ts \
  src/features/groups/CrewVoiceComposer.test.ts \
  src/features/plugins/PluginSkills.test.ts \
  src/features/plugins/PluginsScreen.test.ts \
  src/features/plugins/SkillLibrary.test.ts \
  src/features/plugins/pluginConnectors.test.ts \
  src/features/plugins/pluginSkillsTypes.test.ts \
  src/i18n/language.test.ts \
  src/i18n/mobileRuntimeMessages.test.ts \
  src/lib/loadEnv.test.ts
```

恢复 Auth 页面原有格式、避免全文件重排后，补跑 `pnpm exec tsx --test src/features/auth/AuthHomeScreen.test.ts src/components/settings/SettingsList.test.ts`，39/39 通过。该结果与上面的 675 有重叠，不额外累计。另有直接语音契约/客户端/滚动集成检查 252/252，以及独立 reviewer 的菜单 13/13、dictation/Crew 93/93、i18n 64/64、滚动 22/22；这些也不重复相加。

实际静态命令在工作树根目录执行：

```bash
python3 ~/.codex/skills/frontend-incremental-pr/scripts/frontend_change_scope.py --repo . --base upstream/main
node /tmp/nyxid-android-runtime/unify-final-static.cjs
git diff --check upstream/main -- mobile
```

静态脚本针对 `unify-final-manifest.json` 中 41 个 changed/new TS/TSX/JS/JSON 文件：34 个 TS/TSX 通过 `transpileModule` 语法检查，4 个 JS 通过 `node --check`，JSON 可解析，40 个文件通过显式文件参数的 Prettier 检查。`AuthHomeScreen.tsx` 全文件 Prettier 警告在 `upstream/main` 同样存在，保留其 4 行行为差异，没有为格式检查重排整个登录页面；其语法与直接测试通过。语法检查不等于类型检查，没有可靠 affected mobile typecheck，本地跳过，由 GitHub CI 完成。

本地精确命令/manifest/log：`unify-final-command.sh`、`unify-final-manifest.json`、`unify-final-tests.log`、`unify-final-static.cjs`、`unify-final-static.log`、`unified-native-debug.log`。截图/日志包含运行环境或账户内容，未作为仓库产物上传。

## 验证边界

- 真实说话的麦克风输入、用户 ASR、播放和双向语音仍没有通过证据。服务端空闲提醒或 Listening 状态不能替代它们。
- AOSP 镜像没有 RecognitionService，实际 composer dictation 仍需要有识别服务的镜像/真机。
- 三端复用源组件已确认；Android/iPad 已实际打开同一语音界面，iPhone 当前只有启动/登录入口验证。iPhone 登录后语音及未执行的设备路径仍未通过验收。
- 旧矩阵中的 FCM/MFA/approval/machine、离线冷启动和完整横屏/大字等缺项仍保留；这次没有完整重跑所有聚合用例。
- 完整前端 suite、lint、typecheck 和 production build 交由 GitHub CI。没有 release/distribution 验收。

本轮代码质量门禁：**PASS WITH NOTES**；完整语音声学验收与全部移动端用户路径仍未完成。
