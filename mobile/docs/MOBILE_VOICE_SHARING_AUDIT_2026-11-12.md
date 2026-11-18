# iPad / iPhone / Android 语音共用核查（2026-11-12）

本轮以用户已确认的 iPad 私聊实时语音为交互基准：底部固定紧凑条、波形/状态/时间、麦克风或按住说话、结束；实时和最终转写都进入主聊天；偏好从右上角菜单打开，声音和模式变更只影响下一次通话。

“三端共用同一个语音组件”只适用于同一功能的 React Native 界面和通话业务逻辑，不能理解为所有语音功能、原生音频引擎和系统弹窗完全相同。

## 当前实际调用链

```text
iPad / iPhone / Android
  AssistantScreen
    AssistantVoiceConversation       ← 同一紧凑界面、偏好、错误/恢复控件
      useNativeVoice                 ← 同一权限/前台编排、生命周期、账户/聊天隔离
        AssistantVoiceClient         ← 同一服务端会话、控制连接、静音/PTT/结束规则
          NativeVoiceMedia           ← 同一 VoiceMediaAdapter 接口
            OpenAI + iOS: NativeOpenAIMedia / react-native-webrtc
            OpenAI + Android: voiceMediaDocument / WebView WebRTC
            xAI + 全平台: voiceMediaDocument / WebView PCM bridge
  voiceTranscript                    ← 同一 caption 身份合并、实时/最终消息与历史去重
```

可核查入口：[AssistantScreen](../src/features/assistant/AssistantScreen.tsx)、[AssistantVoiceConversation](../src/features/assistant/voice/AssistantVoiceConversation.tsx)、[useNativeVoice](../src/features/assistant/voice/useNativeVoice.ts)、[AssistantVoiceClient](../src/features/assistant/voice/AssistantVoiceClient.ts)、[VoiceMediaBridge](../src/features/assistant/voice/VoiceMediaBridge.tsx)、[createNativeOpenAIMedia](../src/features/assistant/voice/createNativeOpenAIMedia.ts)、[voiceTranscript](../src/features/assistant/voice/voiceTranscript.ts)。

`mobile/src` 没有平台专用的 production `.ios.tsx` / `.android.tsx` 语音文件；`AssistantVoiceConversation` 没有 `Platform`、`isPad`、`isTablet` 或按窗口宽度选择不同语音界面的分支。当前实时通话直接使用 `ChatComposerFrame`，旧的 `VoiceComposerShell` 只被 Crew 路径使用。

## 共用、差异及保留理由

| 范围                                   | 三端实际关系                                                                        | 决定                                                                                      |
| -------------------------------------- | ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| 私聊语音入口、紧凑条、主聊天转写       | 同一屏幕、组件、样式、caption 合并逻辑                                              | 保持共用；不新增手机或 Android UI 副本。                                                  |
| 开始、监听、静音、按住说话、结束、恢复 | 同一 hook / client / 服务端 API                                                     | 保持共用；媒体适配器必须遵守同一控制含义。                                                |
| 通话偏好与账户设置                     | UI 上下文不同，但原先分别拼装偏好，并有首次默认值冲突                               | 本轮改为共享偏好解析规则，沿用 iPad 首次免提默认；保存的选择继续优先。                    |
| iPhone 与 iPad OpenAI 媒体             | 两者完全相同的原生 WebRTC 类与 iOS 音频配置                                         | 保持共用，保留已验收 iPad 路径。                                                          |
| Android OpenAI 媒体                    | 仍是 WebView WebRTC，非 iOS 原生类                                                  | 暂保留已运行验证的引擎；对齐失联/静音语义，不在本轮切换原生栈。                           |
| xAI 媒体                               | 三端共用 WebView PCM 引擎；该协议仅支持 PTT                                         | 保持共用；协议能力差异不能伪装成平台差异。                                                |
| 麦克风权限与前台切换                   | Android 权限 Activity 可能暂时切后台；iOS 状态序列不同                              | 保留有期限的 Android permission handoff；真正后台仍停止录音，不能删除保护来追求相同代码。 |
| WebView 启动焦点                       | Android prepare 前需要 `requestFocus()`                                             | 保留；这是 Chromium 获取录音所需的宿主操作，不改变语音 UI。                               |
| 扬声器/音频会话                        | iOS 原生 AudioSession 配置由 `withVoiceAudio` 插件设置，iPhone/iPad 同一配置        | 保留 OS 适配。不要把 iOS AudioSession API 搬到 Android。                                  |
| 原生媒体结束 drain                     | 原生引擎可在隐藏 WebView 卸载后继续有期限的控制收尾；WebView 引擎随宿主卸载必须释放 | 保留宿主生命周期差异；用户侧结束/静音规则共用。                                           |
| 屏幕宽度、安全区、键盘                 | 同一响应式布局；系统键盘避让模式随 OS 调整                                          | 保留尺寸/安全区适配，手机不复制 iPad 固定宽度。                                           |
| 三个点菜单、系统权限弹窗               | 相同菜单动作，但原生容器外形由 OS 决定                                              | 保留系统样式；不为像素一致重做已验收 iPad 菜单。                                          |
| 输入框听写                             | 私聊和 Crew 共用 `useComposerDictation` / `ComposerDictationFrame`                  | 保持共用。听写写入可编辑草稿，按发送才发送，与实时通话分别处理。                          |
| Crew 语音                              | 三端共用 `CrewVoiceComposer`，但它不是私聊 realtime 组件                            | 保留独立发送/收件人/TTS 编排，准确记录其展示尚不同于私聊。                                |

Android 可以研究复用 `NativeOpenAIMedia`；当前 `createNativeOpenAIMedia` 中的 iOS 限制表达的是原生路由验证边界，**不代表 Android 技术上只能用 WebView**。直接删除平台判断还需证明 Android 原生音频路由、权限、收放音、PTT、后台/中断和结束清理全部可用。现有 Android WebView 已有真实语音路径证据，本轮先统一用户行为和共用业务规则，避免以未经验证的媒体替换影响已可用流程。

## 本轮修复的实际差异

### 一份偏好规则，Settings 与通话一致

原先通话首次默认 `automatic`，账户设置首次却默认 `push_to_talk`。同一新用户不保存设置与保存设置后，会从不同默认模式进入通话。

共享解析器放在 [voiceSelection](../src/features/assistant/voice/voiceSelection.ts)，通话和 [voiceSettingsModel](../src/features/account/voiceSettingsModel.ts) 都使用它：统一服务/连接/凭据来源/模型、声音、输入模式、语言和完成通知字段；非 Grok 首次沿用 iPad 的 `automatic`，Grok 保持 PTT。保存的 PTT、声音、语言和通知不被重置；已移除的声音仍可见并要求显式替换；不可用或缺失的保存来源不会静默换成另一个来源。

### Android 媒体行为向 iPad 对齐

[voiceMediaDocument](../src/features/assistant/voice/voiceMediaDocument.ts) 原先只在 peer `failed` 时结束，未覆盖 ICE 终态、data channel 错误/关闭、远端音轨结束；iPad 原生适配器已覆盖。这会使音频不可用后界面仍可能显示通话进行中。本轮补齐相同终止条件，并覆盖旧会话回调与主动结束清理，不把它们误报成新通话故障。

原先 WebView 音量表仍读取被关闭的输入或被静音的输出；iPad 原生适配器会将对应信号归零。本轮按相同 mic / speaker 开关过滤音量事件。音量只驱动波形/播音状态，仍不创建 ASR 消息或额外 Transcribing 占位。

## 不应混为一谈的其它入口

- **普通听写**：写草稿、可编辑、手动发送；不应改成实时通话自动发送。iOS Speech Recognition 授权、Android RecognitionService 可用性、iOS audio-session release 各自保留在同一个 hook 的系统适配中。
- **Crew voice**：native ASR → 带 recipient / authority 保护的普通群消息 → `expo-speech` 朗读。它的服务端语义不同，不能用私聊 realtime client 替代；当前三端都采用同一套 Crew 大面板/收起设计。其展示可以后续复用紧凑控制，但必须保留收件人、暂停/重试和转写归属，不能简单固定 minimized 导致失去操作入口。本轮没有改动用户尚未重新确认的 iPad Crew 设计。
- **历史残留**：`withoutLiveUserSpeech` 与 `VoiceAura` 没有 production 调用；它们不决定当前任一平台的实时通话展示，不能用它们判断当前 Android 仍在跑旧大面板。
- **共享的本地化遗漏**：Crew / 旧 shell / 听写部分直接渲染的状态与无障碍文案尚未全部走 `t`；iOS speech permission 文案仍称 iPhone。这是共用代码的遗漏，非 Android 特有实现，本轮聚焦偏好/媒体行为，未扩展成全语音文案改造。

## 验证

### 聚焦自动化与独立审查

修复前新增媒体回归：58 项中 49 通过、9 失败，覆盖终态与 mic/speaker 音量门控。修复后 document 测试 58/58 通过。最终在与 CI 同一主版本的 Node 22.23.4 运行以下 13 个依赖相关文件，**494/494 通过，0 failed / cancelled / skipped**。从 `mobile/` 执行：

```bash
/Users/abigaildeng/.npm/_npx/52027bd8fc0022aa/node_modules/node/bin/node --import tsx --test \
  src/features/account/voiceSettingsModel.test.ts \
  src/features/account/VoiceSettingsScreen.test.ts \
  src/features/assistant/voice/voiceSelection.test.ts \
  src/features/assistant/voice/AssistantVoiceConversation.test.ts \
  src/features/assistant/AssistantScreen.voice.test.ts \
  src/features/assistant/voice/voiceMediaDocument.test.ts \
  src/features/assistant/voice/VoiceMediaBridge.test.ts \
  src/features/assistant/voice/NativeOpenAIMedia.test.ts \
  src/features/assistant/voice/AssistantVoiceClient.test.ts \
  src/features/assistant/voice/useNativeVoice.test.ts \
  src/features/assistant/voice/useNativeVoice.readiness.test.ts \
  src/features/assistant/voice/useVoiceLevels.test.ts \
  src/features/assistant/voice/voiceTranscript.test.ts
```

其中包含 Settings 与实际组件 start payload 的首次偏好一致性（OpenAI/Grok）、保存的 PTT/失效声音、下一次通话才生效，以及 peer/ICE/channel/remote-track 终止、短暂断开恢复、主动结束、旧会话回调隔离、关闭输入/静音播放的音量事件。

独立 reviewer 检查全部差异及调用方，没有确认问题，并独立运行其中 5 文件 **177/177**；该批与 494 项重叠，不重复计数。没有改变紧凑条 JSX/样式或 iPad 原生适配器。

改动文件静态检查在仓库根目录执行：

```bash
python3 /Users/abigaildeng/.codex/skills/frontend-incremental-pr/scripts/frontend_change_scope.py --repo . --base a4e943d18
node /tmp/nyxid-voice-sharing-static.cjs
git diff --check a4e943d18 -- mobile
```

8 个变更 TS/TSX 文件通过语法检查，7 个格式干净；`voiceSelection.test.ts` 的 3 组格式偏差与基线 `a4e943d18` 完全一致，无新增格式偏差。语法检查不等于完整类型检查。没有 repository-native affected mobile typecheck，本地跳过；完整 suite / lint / typecheck / production build 交给 GitHub CI，没有额外跟踪 CI。

日志留本机：`/tmp/nyxid-voice-sharing-media-before.log`、`/tmp/nyxid-voice-sharing-focused-node22.log`、`/tmp/nyxid-voice-sharing-static.log`、`/tmp/nyxid-voice-sharing-format-baselines.json`。

### 本轮设备回归

使用同一 Metro 8081 源码、保留现有 Google 会话，没有登录 Apple，也没有复制登录凭据。

| 设备                               | 本轮实际观察                                                                                                                                                                                                                                                                                          | 边界                                                                                                                                         |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| iPad Pro 13-inch M5 / iOS 26.3     | Home 进入同一紧凑条并达到 Listening；Mac 扬声器播放测试句，经实际模拟器麦克风识别 `17 + 25`，主聊天回复 `42`；静音变为 Microphone paused。                                                                                                                                                            | 随后 Mac 自动锁屏，CUA 明确拒绝继续操作。恢复与正常 End 未完成，已终止 iPad app 释放本地媒体；不将它们计为通过。没有新增 iPad 输出录音证据。 |
| Android Pixel 7 / Android 14 API34 | 热更新后的旧聊天先停留在 loading；保留登录冷启动后正常恢复 Home。重新进入紧凑通话到 Listening，实际麦克风输入 `17 + 25` 转写并回复 `42`；15 秒输出样本为 14.944 秒、RMS 18.367、peak 636，证明该样本非静音；关闭输入后波形静止，`5 + 6` 未新增转写，状态收敛为 Microphone off；恢复后显示 Listening。 | 恢复后的另一短句未在静默结束前得到转写，这次不计为有效输入通过。通话 quiet-ended，receipt 4:05；点击 Close 恢复文字输入。后续定向复测如下。  |
| iPhone 17 Pro / iOS 26.3           | 本轮未执行新的设备回归。                                                                                                                                                                                                                                                                              | Mac 锁屏后不能操作 iOS UI；共用源码与已通过的自动化不替代本轮真机/模拟器验收。                                                               |

后续 Android 定向复测保留前面失败/中断记录，不覆盖它们：第二次进入可到 Listening，但在等待自动化状态期间静默关闭（receipt 3:01，未播放测试句，不计恢复输入通过）。随后点击 Try again 进入新通话，快速播放 `17 + 25` 得到一轮转写/`42`；确认 Microphone off 后恢复到 Listening，再次播放同一句，出现第二条新的转写和 `42`，证明该次静音恢复后的采集有效。点击一次 End 后，receipt **2:55** 为 The audio call ended，紧凑语音条消失、文字输入框恢复。最终没有留下活动通话，Android host microphone 保持开启。

对应后续证据：`sharing-android-fast-call-answer`、`sharing-android-fast-muted`、`sharing-android-fast-unmuted`、`sharing-android-fast-unmuted-answer`、`sharing-android-explicit-ended`。这些结果仍不替代 speaker mute 的本轮声学测量、PTT 或故障注入。

Android 在有效问题前还出现了短 ASR 噪声文本；这些是实际 provider captions，没有重新引入音量驱动的合成占位。不声称开放麦克风下 ASR 永不误识别环境声。测试句使用 `afplay` → Mac 扬声器 → host microphone → 模拟器，未使用有已知宿主崩溃风险的 gRPC audio injection。

本机证据目录 `/tmp/nyxid-android-runtime/` 中的 `sharing-*` 截图和音频仅保留本机；包含账户/对话，不随 PR 上传。终态错误注入目前由执行真实媒体引擎的 VM 回归覆盖，未在三台设备逐一进行网络/原生音轨故障注入。PTT、设备中断、声音路由、横竖屏和完整偏好矩阵不因本轮局部回归被标记为全通过。

本轮质量门禁：**PASS WITH NOTES**，没有未解决的确认代码审查问题；设备回归存在上述边界。历史三端端到端证据与未覆盖项继续以 [端到端报告](MOBILE_VOICE_END_TO_END_2026-11-12.md) 为准，本次源码共用审计不能替代完整设备验收。
