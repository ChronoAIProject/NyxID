# NyxID React Native 性能监控与排查手册

适用范围为 NyxID 的原生 Android / iOS 应用。本分支区分冷启动和热启动，分别采集进入前台、首帧和关键输入可用的耗时，同时采集聊天响应、观测帧率、JS 长任务或事件循环延迟，并在构建时记录产物大小。正式版内存趋势直接看 Apple 和 Google 的商店报告。下面的看板配置、告警阈值和教学数据分别用于配置与读图。

## 1. 一个问题该去哪里看

在 Web 上，浏览器提供绘制、交互等计时，采集器把数据发往分析平台，再由看板展示。RN 原生页面由 Android / iOS 绘制。要回答同样的体验问题，工程师需要从操作系统、RN 代码或构建工具拿到相应观测值。PostHog 能分析发给它的数值。它的 Web Vitals 面板适用于网页，包括单独接入网页采集的 WebView 内容。[PostHog Web Vitals](https://posthog.com/docs/web-analytics/web-vitals) · [RN 性能指南](https://reactnative.dev/docs/0.83/performance)

| 想知道什么 | 数据从哪里来 | 日常去哪里看 | 有异常时怎么查 |
|---|---|---|---|
| 上架版本是否启动变慢、无响应变多 | 平台在符合条件的设备上收集的数据 | Google Play Console → Android vitals；Xcode Organizer → Metrics，按需使用 MetricKit | Android Macrobenchmark / Perfetto；iOS Instruments → App Launch、Time Profiler |
| 用户要多久才能打字、看到回复，哪个版本受影响 | RN 客户端在指定时刻计时，按本机许可直接发往 PostHog | PostHog 按版本、平台、场景看趋势和样本数 | 在真机复现，给网络、JS 和原生显示分别计时 |
| 滚动或键盘动画哪一帧卡住 | Android 帧截止时间、iOS 动画停顿；开发包另提供 JS/UI FPS | 实验室看系统轨迹；需要线上场景趋势时把原生统计按一次交互汇总 | Android Perfetto FrameTimeline；iOS Instruments → Animation Hitches；针对 JS 更新用 React Profiler |
| 安装或升级要多下载多少 | 产物字节数、按设备交付的包和商店估算 | CI 构建报告、Play Console、App Store Connect / App Thinning Size Report | 比较相同配置下的 Hermes 业务包、原生库、图片和字体 |
| 新版本是否更吃内存，用户是否因为内存紧张被中断 | Apple 和 Google 汇总的正式版数据 | iOS 看 Xcode Organizer → Metrics，Android 看 Play Console → Android vitals | 用 Instruments 或 Android Studio Memory Profiler 重复长聊天、打开图片和退出页面，观察占用能否回落 |

Android vitals 和 Apple 的平台报告各有分发、用户共享设置和聚合条件。侧载 QA APK 或 Ad Hoc IPA 的日常验证以真机本地数据为主。不同来源的分母和刷新频率应写在图表描述里。[Android vitals](https://developer.android.google.cn/topic/performance/vitals?hl=zh-cn) · [Apple MetricKit](https://developer.apple.com/documentation/metrickit)

如果团队以后需要从一个线上慢请求点进去，串起页面、HTTP、原生调用和堆栈，再评估应用性能监控平台（APM），例如 Sentry Performance 或 Firebase Performance Monitoring。选型时用一个真实问题验证它能自动采到哪几段、需要多少手工埋点、能否遵守 NyxID 的同意与撤回机制。第一轮可以直接用现有 PostHog 加系统工具。

## 2. 把“慢”写成能复测的定义

### 启动

- **首帧**回答“多久看到第一个应用画面”。Android 的 TTID 由系统测量。冷启动需要新建进程，暖启动会复用部分状态，例如进程还在但 Activity 要重建；热启动通常可复用内存中的 Activity。iOS 将新进程启动和从后台恢复分别统计。应用内首次执行 JS 时，原生加载已经开始。
- **可操作**回答“多久能执行来 App 的第一件事”。本地专项测试可选择两个终点。未登录时登录控件能交互；已登录进入聊天时启动遮罩已消失、输入框可见且已能接收输入。导航 ready、组件第一次布局、真实键盘弹出分别代表不同阶段。用一次自动输入或测试员实际输入校准终点，避免把动画中仍被遮住的控件判成可用。
- **计时**从平台提供的原生启动时点出发，在同一单调时钟下记录间隔。Android 可在上述条件成立时调用 `reportFullyDrawn`，由 Macrobenchmark 读取 TTFD；iOS 用启动指标加业务 signpost。只拿得到 JS 计时的版本可以报告“JS 初始化到可输入”。后台恢复用后台到前台的真实转换单列，排除权限弹窗、控制中心造成的短暂失焦。
- **项目排查点**依次观察域配置、字体、会话恢复、导航和首屏就绪。现有字体等待有 8 秒兜底，可记录触发次数。完成启动才能发出的业务事件会漏掉中途退出、无响应或崩溃的启动，读图时并排看平台启动与稳定性数据。[Android 启动定义](https://developer.android.google.cn/topic/performance/vitals/launch-time?hl=zh-cn) · [Apple 启动耗时](https://developer.apple.com/documentation/xcode/reducing-your-app-s-launch-time) · [NyxID 字体和启动流程](https://github.com/ChronoAIProject/NyxID/blob/9e25a66955abe4a0d8d685392fb4b60d77292e55/mobile/src/app/App.tsx)

### 掉帧和点击反馈

60 Hz 屏幕两帧约隔 16.7 ms，120 Hz 约隔 8.3 ms，设备也可能动态调整刷新率。平均 FPS 可能几乎不动，用户仍能感觉到键盘弹出的一瞬间卡住。Android 的 Macrobenchmark FrameTimingMetric 可按每帧实际截止时间记录超时，优先看慢帧占比、超时长度和连续慢帧。iOS 看动画 hitch time ratio，即动画停顿时间相对于观测动画时间的比例，保留工具显示的单位。两种定义分别画图。[Android 帧指标](https://developer.android.google.cn/topic/performance/benchmarking/macrobenchmark-metrics?hl=zh-cn) · [Apple 响应性分析](https://developer.apple.com/documentation/xcode/analyzing-responsiveness-issues-in-your-shipping-app)

对点击另测“按下到可见反馈”。例如点发送后，用户应很快看到消息气泡或发送中状态，即使服务端仍在计算。Dev Menu 的 JS/UI FPS 能帮助选择分析工具；JS 的 `requestAnimationFrame` 循环只能观察 JS 回调节奏。要确认哪帧真正错过显示期限，需要原生帧数据。

### 聊天响应

从点击发送起，在同一轮累计记录四个时点，分别是响应头返回、收到首段可显示文字、该文字出现在屏幕、轮次结束。工具状态、心跳和审批卡可能先到。把普通文本、纯审批结束、超时、断流、取消标成不同结果。React 提交或 `onLayout` 是可实现的近似终点；如果仅能取得这一层，则图名标为“首段文字提交”，在真机用原生绘制轨迹或录像校准后再解释为可见延迟。

例如某一轮在 180 ms 收到响应头，800 ms 收到文字，1,650 ms 才把文字显示出来。这一轮中有 850 ms 花在文字到达之后，优先查 JS 更新、Markdown 解析和列表绘制。若文字在 1,600 ms 才到、1,650 ms 已显示，先看请求、代理和模型首输出。这个差值要在**同一轮**计算后再汇总；把两张曲线的 P95 相减无法还原单轮差值。[NyxID 流式请求](https://github.com/ChronoAIProject/NyxID/blob/9e25a66955abe4a0d8d685392fb4b60d77292e55/mobile/src/lib/api/assistantApi.ts) · [服务端阶段追踪](https://github.com/ChronoAIProject/NyxID/blob/9e25a66955abe4a0d8d685392fb4b60d77292e55/scripts/turn-trace.py)

### 包大小与稳定性

每个版本分别记录 CI 生成的 APK/AAB/IPA 字节数、指定设备经过拆分和压缩后的下载量、设备安装后的占用。每版用同一架构、签名方式、构建配置比较。Android 可对相同设备规格使用 `bundletool get-size total`，iOS 查看 App Thinning Size Report。同为 ARM64 的 NyxID QA build 8 到 9 从 63,062,771 增至 63,071,651 字节，增长 8,880 字节，Hermes 业务包的增量也是 8,880 字节。这是历史 QA 产物的实测示例，不是本次性能采集接入的包体积验证。[Android bundletool](https://developer.android.google.cn/tools/bundletool?hl=zh-cn) · [Apple 包体积](https://developer.apple.com/documentation/xcode/reducing-your-app-s-size)

改动移动端打包输入并向 `dev` 提 PR 后，在 PR 的 **Checks → Mobile package size → Details → Summary** 查看本次构建。摘要按平台显示文件大小、精确字节数和预算。Android 构建多架构 Release AAB，iOS 把含 Watch 应用的未签名真机 Release 归档压成参考 IPA。比较两次 PR 的 iOS 体积时，选相同类型的参考包。正式出包走 `pnpm build:android` 或 `pnpm build:ios`，构建日志也会记录最终 AAB 或已签名 IPA 的大小。在 GitHub Actions 内运行这两个命令时，同一份结果会写进运行摘要。

初始提醒线为 Android AAB 110 MiB、iOS 未签名参考 IPA 30 MiB、已签名 IPA 25 MiB。ARM64 单架构 QA APK 可在 `mobile/` 运行 `node scripts/ci-package-size.js android-arm64-apk /path/to/arm64.apk`，提醒线为 70 MiB。超过预算时 Actions 发出警告，构建可继续。这些值是团队可在 `mobile/scripts/ci-package-size.js` 调整的预算。报告直接读取构建产物，查看时无需配置 PostHog。

稳定性与速度一同观察。Android 看用户感知 ANR（应用无响应）和崩溃，iOS 看 hangs 和崩溃。终止进程的会话可能来不及发最后一个业务事件。若看“无崩溃会话占比”，先确认数据源确实具备同口径的受影响会话数与会话总数。耗电和发热放在连续语音等长时间工作场景。

### 正式版与测试包的内存怎么查

- **正式版。** iOS 在 Xcode Organizer 的 Metrics 里选 Memory，在同一设备类型上对比新旧版本。Android 在 Play Console 的 Android vitals 里按版本和设备内存档位看内存占用，再看每天遇到可感知低内存终止的用户占比。两个平台各用自己的口径对比版本。参与设备太少时，商店可能暂时没有数据。[Apple 线上指标](https://developer.apple.com/documentation/xcode/analyzing-the-performance-of-your-shipping-app) · [Android 内存占用](https://developer.android.com/google/play/vitals/memory-usage) · [Android 低内存终止率](https://developer.android.com/google/play/vitals/lmk)
- **侧载 QA 包。** 使用可分析的构建，在真机上打开 Instruments → Allocations 或 Android Studio → Memory Profiler。从空闲开始录制，反复进入同一段长聊天、打开同一张图片再退出。记录每次峰值和退出后稳定的占用。若后者逐轮上升，再检查哪些对象仍被保留。[Apple 内存分析](https://developer.apple.com/documentation/xcode/gathering-information-about-memory-use) · [Android 内存分析工具](https://developer.android.com/studio/profile/memory-profiler)

目前直接查看商店报告。以后需要在 PostHog 统一看发版数据时，可启用商店连接器，只同步所需的版本汇总表。同步行数计入数据仓库用量，商店样本不足时同步后也会缺数。[PostHog 的 Apple 连接器](https://posthog.com/docs/data-pipelines/sources/app-store-connect) · [PostHog 的 Google Play 连接器](https://posthog.com/docs/data-pipelines/sources/google-play-console)

## 3. NyxID 的 PostHog 看板怎么搭

本版设计采用 **RN 客户端 → PostHog**。App 在本机测量，使用 `posthog-react-native` 的即时发送路径向项目的 `/batch/` 接口发送四个自定义事件。客户端配置取自目标 PostHog 项目的公开 Project Token 和接收事件的 host。个人 API key 只用于有管理权限的操作，不能打进 App。构建脚本从构建机直连同一项目，另发一类产物事件。现有产品分析使用 NyxID relay。[PostHog RN SDK](https://posthog.com/docs/libraries/react-native) · [现有产品分析约定](https://github.com/ChronoAIProject/NyxID/blob/9e25a66955abe4a0d8d685392fb4b60d77292e55/docs/TELEMETRY.md)

**构建配置**。在对应环境的移动端构建配置中设置 `DEV_PERFORMANCE_POSTHOG_TOKEN` 与 `DEV_PERFORMANCE_POSTHOG_HOST`，或对应的 `PROD_` 变量。Project Settings 里的公开项目 token 以 `phc_` 开头；host 使用实际 HTTPS 接收域名，例如美国区 `https://us.i.posthog.com` 或欧洲区 `https://eu.i.posthog.com`。每个环境独立配置，留空时隐藏性能开关。首次安装需要包含新原生模块的构建。

本机开关在设置的隐私页，默认关闭。选择绑定本机配置的接收项目。撤回会使仍在测量或准备发送的样本失效；已经交给网络的请求可能送达。性能客户端使用内存匿名身份、关闭回放、自动采集和地理位置解析，每个事件即时发送一次，离线失败直接丢弃。跨设备撤回不在本版范围内。开发环境抽取所有符合条件的交互，生产环境对每次符合条件的交互独立抽样 10%。每次开始时决定是否抽样，最多同时发送 8 条；抽中的事件仍可能因关闭、退出或传输失败而缺失。

每条事件包含 `surface=mobile`、`environment`、`platform`、原生 App 版本 `client_version` 和构建号 `build_number`。

运行时事件还会附带本次进程内的匿名 `distinct_id`、事件 UUID、时间戳和客户端库版本。匿名身份只保存在内存，应用进程重启后更换，不绑定 NyxID 账号，也不发送持久设备标识。接收服务仍可看到网络连接的 IP；关闭 GeoIP 解析不等于隐藏连接 IP。关闭分享停止未来观测并使未发送样本失效，不删除已经接收的事件；已接收数据的保留周期由项目配置决定。

| 事件 | 一条代表什么 | 字段和测量边界 |
|---|---|---|
| `mobile.perf_launch` | 一次已授权、被抽中的启动达到一个阶段 | `startup_kind` 为 cold 或 hot。三个阶段共用原生单调时钟起点，分别上报 `app_active`、`first_frame`、`interaction_ready` 的 `duration_ms`。用 `origin_source`、`measurement_source` 和 `target` 区分平台、终点和控件。 |
| `mobile.perf_chat_turn` | 一轮通过输入框发送的聊天 | `headers_ms`、`first_text_ms`、`first_text_commit_ms` 均从发送操作起算；`duration_ms` 在轮次终态时冻结。 `outcome` 为 completed、no_text、cancelled、error 或 timeout。能看到正文但结束后两秒仍未观察到提交时，省略提交耗时。|
| `mobile.perf_frames` | 聊天页内一次可观测的拖动或流式输出窗口 | `scenario` 区分 chat_drag、chat_drag_during_stream 和 chat_streaming。`measurement_source`、`duration_ms`、`sample_count`、`over_budget_count` 及 `interval_p50_ms`、`interval_p95_ms`、`interval_max_ms` 描述这一窗口。Android 采 Window FrameMetrics，可用时含 `dropped_callback_count`。iOS 采 CADisplayLink 回调间隔，超预算回调是显示节奏的近似值。Apple 官方动画停顿数据在 Instruments 或 Organizer 核对。|
| `mobile.perf_js_tasks` | 每次进入前台或在前台开启分享，抽取一次约 10 秒的连续窗口 | `rn_longtask` 表示 RN 能观测已完成的 ≥50 ms JS 任务，报告 `task_count`、最长任务 `max_task_ms` 和超过阈值的累计 `blocked_ms`。如运行时未提供该能力，每 250 ms 测一次定时器延迟，来源为 `event_loop_lag`，单列 `stall_count`、`max_lag_ms`。延迟次数与长任务次数口径不同。窗口中途进后台或撤回即丢弃。 |
| `mobile.perf_artifact` | 构建工具读取最终产物后发送一次 | `size_bytes` 是文件字节数。按 `platform`、`build_number`、`build_configuration` 和 `artifact_kind` 分组；同构建重报时按最新值显示。APK、AAB、IPA 分别比较。iOS 模拟器的 `simulator_app_uncompressed` 是 .app 内普通文件的逻辑大小之和，跳过符号链接。 |

**启动图应怎样读**。同一平台、构建号和启动类型下，三张图回答三个不同的问题。

| 看板 | 起点到终点 | 读数边界 |
|---|---|---|
| 冷启动与热启动 | Android 冷启动从系统的进程启动时点计，热启动从同一 Activity 经后台重新进入 started 状态计，到 Activity 恢复。iOS 冷启动从 didFinishLaunching 计，热启动从 willEnterForeground 计，到 didBecomeActive。 | iOS 冷启动读数从回调开始，系统加载和 pre-main 用 Instruments 单独核验。Android 排除主线程已经空闲才创建首个 Activity 的预启动进程。同一 Activity 的短暂失焦可继续原有观测，只有真正退到后台再返回才开启一轮热启动。 |
| 首帧展示 | 从同一次启动的起点计到应用窗口的第一个可观测帧。Android 用窗口帧指标回调；iOS 用窗口提交后的一次显示节奏回调估算。 | 这是客户端可观测的显示时点；屏幕上是否有业务内容、物理像素哪一刻点亮，可对照系统轨迹或录像。 |
| 关键交互可用 | 从同一起点计到当前路由无遮挡、已启用的登录邮箱输入框或聊天输入框布局完成并通过就绪检查。 | 每个启动最多一个目标。无对应输入框的页面没有该样本。测试时应实际点击控件并输入字符，检查其在同一页面响应；人的思考或打字等待时间不计入线上耗时。 |

按平台、阶段和 `target` 比较版本的 P50、P95 和样本数。进入前台、绘出首帧和输入可用各有自己的样本数。首次在设置里开启分享，只对后续符合条件的启动生效。

**帧率图**读取 `mobile.perf_frames`。窗口的 `observed_fps` 为实际观测帧回调数除以经过的秒数。Android 窗口只在应用产生帧时收到回调，静止画面的低值也可能代表没有重绘需求。iOS 记录 CADisplayLink 回调频率。比较同平台、同场景和同构建类型，同时看慢帧占比。

**包大小图**从标准 Android AAB 和 iOS IPA 发布脚本自动取数，只传构建元数据；独立于用户的本机开关，失败不阻断出包。QA 可在 mobile/ 中运行 `APP_ENV=dev node scripts/performance-artifact.js --kind apk --configuration debug --version 1.0.4 --build 112502 --file /path/to/build.apk`。同一种产物按构建号看变化，再用分发平台提供的拆包报告估算用户下载量。

**第一张图的具体操作**

1. 安装配置了 QA 项目的新包后，用测试环境、已在本机开启性能分享的设备完成一次指定动作。在 PostHog 的事件列表里找到样本，核对耗时是数值、平台和构建号正确。用本地时间记录核对数量级，再做一次取消和一次无文本结束，检查结果枚举、耗时缺失情况。
2. 打开 **Product analytics → Trends**，选择 `mobile.perf_chat_turn`，使用数值属性聚合，属性选 `first_text_commit_ms`，聚合选 **95th percentile**。过滤 `surface=mobile` 和目标环境，保留有该属性的轮次，按 `platform` 拆分，固定一个平台后再比较 `build_number`。保存为“发送到首段文字提交 · P95”。图表描述写明“在本机开启性能分享的抽样交互”。[PostHog Trends 聚合](https://posthog.com/docs/product-analytics/trends/aggregations)
3. 同页再放 P50、含该时点的样本数，以及按 `outcome` 拆分的全部已记录轮次数。P50 表示中位体验；P95 表示约 95% 的已观测耗时在此值以内。只有两三个请求时，保留原始值更容易理解。首段文字缺失不能填 0，否则一批超时就可能让图看起来变快。
4. Android 帧图用两个 Sum 序列及公式 `100 × sum(over_budget_count) / sum(sample_count)`，按场景筛选 Android 的观察样本，排除零分母。iOS 另建“显示回调超预算占比”图，选择 `ios_display_link` 数据源，使用同一计数公式。不要把每个窗口的 P95 再平均后当作所有帧的 P95。
5. 上报汇总放在测量窗口之后，防止监控自身影响启动或流式输出。每次采样仅传预定义枚举和耗时，不增加聊天原文、完整 URL、token、设备或聊天标识。记录本机许可覆盖范围、抽样策略及其变更日期。看板的“样本数”指成功入库的观测量。release 和 QA 应使用可区分的环境或发布渠道。

**直连验收**。QA 包使用测试环境和已知构建号。在设备默认关闭时确认没有性能请求；开启后保持前台至少 12 秒，再执行一次聊天和滚动，检查请求直接发往配置的 PostHog host，并在事件详情核对名称、数值类型和平台字段。断网时触发一次事件，先在本机关闭分享再恢复网络，验证待发事件不会补送；重启后检查关闭状态仍然生效。

**验收口径**。使用真实安装包，记录原生版本、构建号和各阶段的 UTC 时间，在 PostHog 按相同维度回查。对照本地文件字节数核验产物事件；查看运行时事件的来源、窗口数及单位。模拟器适合验证开关、采集和报表，真机发布包用于建立性能基线。

## 4. 新手可独立完成的第一次真机实验

**目标**是假设“长聊天一边出字一边翻历史时，滚动更容易卡”。准备同一台真机、同一构建、同一份脱敏长对话，固定字体、屏幕刷新率设置和网络。记录系统版本、设备、包构建号、温度或是否明显发热。模拟器适合先走通路线，真机提供结论。

1. 开发包打开 Dev Menu → Perf Monitor。分别做三项各约 30 秒的动作，分别是停止生成后上下滑动、保持静止只看流式输出、边输出边向上翻历史。每次记录卡顿发生在哪个动作、JS/UI FPS 如何变化。输出快慢会影响结果；制作可比较的前后测试时，用同样的消息片段和节奏回放。
2. 同机换成 Release 或接近 Release 且允许 profiling 的包，关闭开发工具叠层，重复三项。Android 先看系统帧统计，必要时录 Android Studio System Trace / Perfetto，Android 12 及以上检查 FrameTimeline。iOS 在 Instruments 选设备和进程，录 Animation Hitches，标下卡住的时刻，再用 Time Profiler 看相应时间段的主线程工作。
3. 若 JS 侧长时间繁忙，复现时用 React Profiler 看一次文本更新带动多少历史行重渲染，再看 Markdown 解析。若 JS 平稳而原生呈现超时，看布局、图片解码、渲染线程和键盘动画。记录证据后只修改被证实的环节，在相同条件下重跑。

**Android 快速操作**。先用 `adb devices -l` 选择设备，用 `adb -s 设备序列号 shell pm list packages nyxid` 确认安装的 application ID。以下两处替换为本次设备和包名。运行冷启动命令会停止并重新打开选中的应用，保留其登录和应用数据。

```sh
device='替换为 adb devices 列出的设备序列号'
pkg='替换为目标 application ID'

# 从桌面入口冷启动进程，保留原始输出
adb -s "$device" shell am start -S -W \
  -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -p "$pkg"

# 每次只录一个场景。reset 只清该包的帧统计
adb -s "$device" shell dumpsys gfxinfo "$pkg" reset
# 现在在设备上执行固定的 30 秒场景
adb -s "$device" shell dumpsys gfxinfo "$pkg" framestats > nyxid-frames.txt
```

启动练习可做十次保留中位数和范围，观察系统输出的启动分类和耗时。`am start -W` 的等待时间无法证明输入框已经可用。帧统计文件保留构建号和场景以便比较。现有历史 Perfetto 键盘轨迹可作为阅读练习，用同一套动作重新采的轨迹做当前版本验收。[Android Macrobenchmark](https://developer.android.google.cn/topic/performance/benchmarking/macrobenchmark-metrics?hl=zh-cn)

**iOS 等价练习**。在 Xcode 的 Product → Profile 中选择已连接的真机和可分析的 Release/Profile 构建。冷启动选 App Launch，记录进程启动和第一屏；滚动选 Animation Hitches，选择一次明显的停顿并记录时间范围；同场景复采 Time Profiler 找出耗时调用。每次保留设备、构建、动作、时长和 `.trace`。要校准“能输入”的终点，用相同的自动输入动作或带时间记录的录像对照。

## 5. 从一次实验到每次发版

| 时机 | 执行动作 | 留下什么 |
|---|---|---|
| 开发中有人反馈卡 | 记录页面、具体手势、消息长度、是否流式和可重复条件，用 Dev Menu 缩小范围 | 一条可复现步骤和对应轨迹时间区间 |
| 每次构建 | 按平台和架构保存精确产物大小，与同配置上一构建比较 | 构建号、字节数、增量和主要变化文件 |
| 发布候选包 | 固定真机比较稳定版与候选版，交替运行，分别测启动到可用、长聊滚动、键盘和聊天首文字 | 中位数/范围、各指标分母、最差轨迹、失败次数 |
| 新版开始有用户 | 按平台和构建看 PostHog 趋势、样本覆盖和结果分布，同时看商店内存占用、低内存终止和无响应指标 | 受影响版本、平台、设备档位、渠道和影响范围 |
| 收到告警 | 先查埋点口径、分母、版本流量和设备构成是否改变，再找可复现的慢场景；修复后重做同机对照 | 指标前后值、相同口径的线上复核和残余问题 |

告警从稳定版的自然波动推导。例如团队可试行“相对稳定版本上升至少 20%，且绝对增加至少 200 ms，达到团队约定的最低样本量并持续两个观察窗口”作为**启动 P95** 的候选规则。这是试运行参数；慢帧率有自己的分母和波动，崩溃/ANR 参考各平台口径。样本少的内测以真机逐次结果为主。

**每次验收记录一行即可**

| 场景及起止点 | 设备 / 系统 / 刷新率 | 基线与候选构建 | 次数 / 观察帧 / 失败数 | 前后数值与单位 | 轨迹时间段 | 结论 |
|---|---|---|---|---|---|---|
| 示例：已登录冷启动到输入框接受首个字符 | 填实测条件 | 填两个原生构建号 | 分别填写 | 中位数和范围 | 附文件与时间点 | 改善 / 持平 / 退化 / 数据不足 |
