# [Engineering Mode] v4.2 可恢复隐藏增量 — 独立验收第 1 轮

**结论：FAIL。** 发现 3 项 P2，均需在下一轮修复或提供足以推翻本反例的证据。另有 1 项 P3 非阻塞观察。默认关闭产物的主要展示/请求边界通过；该结论不能覆盖开启后的缺陷。

日期：2026-10-21（Asia/Shanghai）。本轮仅审查 c17f8c8 之后的可恢复隐藏增量，旧 SEO 第二轮通过结论保持绑定旧基线，不作为本轮通过结论。正式计数：本增量第 1 轮，最多两轮。

## 版本与环境

- 基线：`c17f8c8fe0aa23eb2892377af3be1f6c0bf51a52`
- 交验 HEAD：`ed91d302ff560388315b6cc1149491bb9d5beb95`
- CI / 产物工程 SHA：`fd00726ec086ed5cf3d8ad19a20e88a00555a2b0`
- content revision：`49e686770de9ed0f5a924923048c88611956e8d8ae7af93bb73744fa9ba236b1`
- config revision：`a27cd071131edbef9665b638f622bf1a541f90673ee733d077c77f71e2faef27`
- 仓库与分支：`aelf-hzz780/NyxID:codex/website-a-delivery`。工程 SHA 至交验 HEAD 仅变更三份交付文档/证据；六个增量 commits 已独立核对。
- CI：[run 39578181573](https://github.com/aelf-hzz780/NyxID/actions/runs/39578181573)；job `119148848271`；artifact `12335496628`。
- 独立下载 ZIP SHA256：`d0f0bc5adb532428f9022061a9a8b896540a3527d1fcd69d057d1917c57fad32`。
- 本地：macOS 26.6.2 arm64、Node v23.11.0、Python 3.14.2；离线源码函数/表达式探针、HTML DOM 与 CommonMark AST。远端：Ubuntu runner、Node 20 配置、Chromium、Nginx 与 backend fixture，默认关闭配置、单 worker。
- 未改业务代码；只在本独立证据目录写报告和探针。未本地全量构建、未运行浏览器矩阵、未使用 4180/IAB、未启停现有服务、未提交/推送/部署、未向其它 chat 发消息。两位只读 reviewer 分别检查恢复路径和产物/验证，主审复核全部反例。

## 需修复的发现

### V-F01 · P2 · Status 恢复校验接受中文官网首页

[定位](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/features/website-a/release-config.ts:98)。

**预期：**官网首页及其 locale/trailing-slash 别名不能作为状态页。

**实际：**URL https://nyx.chrono-ai.fun/zh 与 /zh/ 配合开关 true、非空 acceptance 可通过；根 / 被拒绝。

**复现：**调用 resolveWebsiteRelease({VITE_WEBSITE_STATUS_ENABLED:"true",VITE_WEBSITE_STATUS_URL:"https://nyx.chrono-ai.fun/zh",VITE_WEBSITE_STATUS_ACCEPTANCE:"review/status"})，返回 footerStatus 为该首页。

**影响：**恢复后 Footer 将中文官网首页标为服务状态；默认 off 不触发。

**修复与复验：**根据官网实际路由归一化首页别名并拒绝，补 /zh、/zh/ 回归；保留真实独立状态域名首页支持。

证据：[independent-probe.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/independent-probe.json), [public-release.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/release/dist/public-release.json), [2026-10-16-website-scope-cut-v4.2.md](/Users/huangzongzhe/workspace/vibeCoding/nyxid/docs/plans/2026-10-16-website-scope-cut-v4.2.md)

### V-F02 · P2 · 恢复方案 CTA 后邀请码嵌入 return_to，认证表单无法使用

[定位](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/features/website-a/solution-activation.tsx:18)。

**预期：**选中方案目标与顶层邀请码同时通过认证，沿用可信 return_to 校验。

**实际：**/solutions/help-desk?code=NYX-ABCDEF12 → /assistant?code=NYX-ABCDEF12&setup=help-desk → /login?return_to=<encoded assistant URL>；LoginPage params.get("code") 为 null。

**复现：**设置 help-desk target 为 /assistant?setup=help-desk；用上述匿名邀请地址生成 CTA。app-entry 与 assistant 的 standaloneAuthBeforeLoad 均只产生顶层 return_to；Login/Register 仅读顶层 code。

**影响：**邀请码并未从所有 URL 消失，但被嵌套后无法预填/传给注册；需要邀请码时访客须重新输入。默认关闭不触发。

**修复与复验：**在官网方案入口或共享认证交接处保留有效顶层 code，并保留完整方案目标；覆盖真实 siteHref 与 router/auth handoff 的匿名→登录→注册路径，不把 siteHref mock 成 identity。不要改变普通登录默认落点。

证据：[independent-probe.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/independent-probe.json), [router.tsx](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/router.tsx), [app-entry.tsx](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/app-entry.tsx), [login.tsx](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/pages/login.tsx), [auth-flow.tsx](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/components/auth/auth-flow.tsx)

### V-F03 · P2 · 关闭组件 bundle 门禁漏检反引号字符串

[定位](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/scripts/public-visibility-contract.mjs:31)。

**预期：**符合实际 minifier 语法的关闭组件残留应被识别并阻断。

**实际：**{"data-website-feature":"newsletter"} 返回 ["newsletter"]；{"data-website-feature":`newsletter`} 返回 []，两者均为合法等价 JS。

**复现：**实际调用 retainedDisabledFeatureCode，visibility 三项 false，传入上面两个对象片段。独立探针结果已保存。

**影响：**默认关闭或只开放另一功能时，此种残留会绕过新门禁。当前下载包独立扫描未检出可操作组件泄漏，不能将门禁缺陷描述为已发生的 UI 泄漏。

**修复与复验：**补充实际压缩产物使用的字符串语法反例并修复检测；可用 AST 或覆盖无插值 template literal 的可靠解析。覆盖三组件及某一其他功能开启的场景。

证据：[independent-probe.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/independent-probe.json), [public-visibility-contract.test.ts](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/test/public-visibility-contract.test.ts), [verify-public-build.mjs](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/scripts/verify-public-build.mjs)

V-F02 额外核对：`router.tsx` 的 `standaloneAuthBeforeLoad` 也只构造顶层 `return_to`；Login/Register 都只读顶层 `code`，AuthFlow 只依据 `initialInviteCode` 预填并在切换面板时传递。证据是完整静态路径和实际函数/表达式探针，**不是已经运行完整真实登录浏览器流程的声明**。

## 非阻塞观察

**V-O01 · P3：默认 chunk 残留三个无网络行为的 adapter schema 初始化。** optional-features-CE3HMhbm.js 中仍执行 email、pending_confirmation、reply 三个 Zod schema 初始化；组件表单和请求函数已剔除。 不构成控件、监听器或请求泄漏，不单独阻塞本轮；不能据此声称所有 adapter 相关字节为零。 如保留“排除 dormant adapters”声明，收紧 tree-shaking；否则将声明限定为无可操作组件/请求实现。

[当前实际 chunk](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/release/dist/assets/optional-features-CE3HMhbm.js)；[源 schema 初始化](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/features/website-a/public-feature-adapters.ts:4)。

## 已核实证据与覆盖限制

- 远端日志确认 30 个测试文件 / 232 tests 通过；`tsc` 和完整 build 成功；90 页产物门禁、219 项 HTTP、隐私日志门禁和 10 项浏览器测试通过，浏览器 0 skipped / flaky / unexpected。这些数值直接来自独立读取的 GitHub 日志和下载产物。
- 离线独立核对 90 页发布路径与 indexable 策略和旧基线一致；90 份 Markdown 与旧产物逐字一致。38 个营销页面 Footer Blog 保留、主导航 Blog 关闭；18 个方案详情原生 disabled；2 个 Contact 页面五字段及无 JS 禁用保持。
- 对 450 个 JS 文件以单引号、双引号、反引号三种形式扫描，未发现三个可选组件的标记。此结果独立于有缺陷的项目 regex；不能扩大为所有相关字节都已消失。
- EN/ZH 首页各两个代码块，Agents 各四个代码块；独立 CommonMark AST 再核对关键正文/代码标记及链接。既有批准 tab 全文没有因新开关被剔除。
- 独立重算 `configRevision`/`contentRevision`，所有 SSR 配置属性与 manifest 一致。CI fixture 覆盖同布尔不同 endpoint；本轮调用实际 boundary 函数，增加同布尔不同 solution target 的拒绝证据。没有将此写成整包混装浏览器测试。
- 8 组合证据是 resolver 与直接组件 fixtures；真实 renderer 覆盖全 off 和三个单项 on。**本轮完整生产构建/浏览器只覆盖默认 off**，真实开放前仍需配置相符的完整 build、目标环境验收和全产物发布。
- 真实 Newsletter、公共 Chat、Status、方案完整任务、候选文章审批均未用 fixtures 冒充通过；部署与真实线上认证未执行。
- 单次 CI lab：`/security`、reduced-motion，CLS `1.9513959385818906`，LCP `56ms`，73 resources / 559843 transfer bytes。仅保留观测，不判性能通过，也不判本增量导致性能回归；field CWV 未测。

## 检查矩阵

合计 **26 项：16 pass、3 fail、4 not-run、3 blocked**。blocked 是明确的外部验收依赖；not-run 是未执行的范围。二者不替代代码缺陷，也不单独构成本轮 FAIL 原因。每项完整 expected / actual / environment / evidence 及四个版本标识保存在 [checks.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/checks.json)。

| ID | 检查 | 状态 | 实际结果 / 边界 |
|---|---|---|---|
| V-C01 | 版本、提交和产物绑定 | **pass** | 六个增量 commits；工程 SHA 到 HEAD 仅三个文档/证据文件；ZIP SHA256 独立计算匹配。 |
| V-C02 | 远端 CI 真实性 | **pass** | 30 files / 232 tests；tsc 与完整 build；90 pages；219 HTTP checks；privacy gate；10 browser passed，0 skipped/flaky/unexpected。 |
| V-C03 | 默认全部关闭 | **pass** | 38 个营销 SSR 页面无三功能；18 个方案详情原生 disabled；CI 浏览时无同源 POST。 |
| V-C04 | 保留源码与开关独立性 | **pass** | 三组件及 adapter 保留；8 组合 resolver/组件 fixtures 已通过；renderer 覆盖全 off 与三个单项 on；非法/缺验收引用抛错。 |
| V-C05 | Newsletter adapter 行为 | **pass** | 仅接受 pending_confirmation；显示未确认；POST 无凭证，10 秒超时/16 KiB 上限；SSR disabled，失败保留输入；fixture 证据不代表真实订阅。 |
| V-C06 | 公共 NyxBot adapter 行为 | **pass** | 空白回复被拒绝、回复按文本渲染、错误保留问题、取消清理；无 cookie/auth/Agent Key；无 mount 请求。 |
| V-C07 | Status 恢复地址校验 | **fail** | V-F01：/zh 与 /zh/ 均被 resolver 接受；/zh 在 manifest 中为首页。 |
| V-C08 | Blog 菜单、Footer 和直达页面 | **pass** | EN/ZH、700/1280 宽度 CI 通过；38 页 Footer Blog 保留；独立对比 90 页发布路径/indexable/Markdown 路径一致。Blog 原有 noindex 策略未被开关改变。 |
| V-C09 | 草稿边界与已批准发布集 | **pass** | publication 输入无 diff；CI publication-footprint 通过；公开路由集合与旧基线相同；新增组件不加载草稿源。 |
| V-C10 | 九个方案默认禁用 | **pass** | EN/ZH 共 18 页独立 DOM 检查通过；辅助提示保留，通用登录和联系入口仍可用。 |
| V-C11 | 逐方案配置和 legacy gate | **pass** | 九个逐项组件/legacy resolver fixtures 通过；目标仅 assistant/dashboard；未将 fixture 目标等同真实业务已完成。 |
| V-C12 | 恢复方案入口的认证上下文 | **fail** | V-F02：方案 CTA 将 code 放入工作区 query，未登录保护仅嵌套进 return_to；LoginPage 读不到顶层 code，切注册丢失邀请预填。 |
| V-C13 | 完整配置与 revision 一致 | **pass** | 独立重算 config/content hash；所有配置属性对齐 manifest；CI 验证不同 endpoint；本轮实际 boundary 函数拒绝不同 solution target。 |
| V-C14 | 已批准 tab 全文与 Markdown | **pass** | 90 份 Markdown 与旧产物逐字一致；EN/ZH 首页各 2 code blocks、Agents 各 4 code blocks；独立 AST markers 与链接检查通过。 |
| V-C15 | 当前默认包的可操作组件排除 | **pass** | 450 个 JS 文件扫描覆盖三种引号形式，未找到可选控件标记；源码/实际 chunk 无可选表单或 adapter 请求函数。仍有惰性 schema 初始化，见 V-O01，不能扩大为零相关字节声明。 |
| V-C16 | bundle 防残留门禁 | **fail** | V-F03：同一有效标记，双引号被识别、反引号版本返回空数组；与当前 Rolldown 使用的字符串语法不兼容。 |
| V-C17 | 文档与恢复/回滚步骤 | **pass** | 计划、review.html 与生成器更新；恢复需验收引用+单项开关+全产物构建发布；回滚只关闭该项并重建；历史图片明确为视觉目标。 |
| V-C18 | 既有默认路径回归 | **pass** | 相应 CI 单元/浏览器用例通过；认证核心无 diff；Contact 五字段及 SSR disabled 独立复核通过。新启用 CTA 的邀请码例外单列 V-C12，不用此通过项覆盖。 |
| V-C19 | 提交范围与构建独立性 | **pass** | 六个 commits；tracked tree clean；diff 不涉及 v4.1；npm build 不调用原型迁移/截图脚本。协调方未跟踪视觉文件不属于本轮代码交付。 |
| V-C20 | on 配置完整生产 build/browser | **not-run** | 本轮远端完整构建仅 all-off；8 组合在 resolver/直接组件，renderer 是 all-off+单项 on；无 on 的生产 bundle/路由/llms/sitemap/browser 组合证据。 |
| V-C21 | 整包 HTML/JS 混装浏览器场景 | **not-run** | 已验证纯 boundary 函数不同 endpoint/target 抛错；未执行整包替换 JS 的浏览器测试，不将纯函数证明写为 E2E。 |
| V-C22 | 真实 Newsletter / 公共 Chat / Status | **blocked** | 未提供真实端点/状态地址及完整验收记录；仅 fixtures；保持关闭。非本轮代码失败原因。 |
| V-C23 | 方案完整真实任务 | **blocked** | 无九方案任一真实端到端任务验收证据；TG 已连接/登录成功不算方案完成。邀请码缺陷本身仍需修复，见 V-F02。 |
| V-C24 | 部署与线上认证 | **not-run** | 本轮未部署，未连接线上会话，未启动/停止现有服务，未访问本机 4180。 |
| V-C25 | 真实性能及 field CWV | **not-run** | 只有 /security 单次 reduced-motion CI lab：CLS 1.9513959385818906，LCP 56ms；不能判性能通过，也不能据此判本增量回归。 |
| V-C26 | 候选文章发布批准 | **blocked** | 本轮未批准任何候选文章；既有批准集及索引策略保留。 |

## 下一轮收口条件

1. 修复 V-F01、V-F02、V-F03，补覆盖反例的针对性测试；V-O01 可修复或准确收窄剔除声明。
2. 提供新 HEAD、对应工程 SHA、content/config revision、CI run 与完整 artifact。若 HEAD 仅增加文档，须保留明确 diff 证明。
3. 复验重点是三个反例、配置恢复边界与相关回归；不用重复无变化的全浏览器矩阵。本轮 4180 与服务不干扰限制继续有效。真实服务依赖仍单列，不以修复工程缺陷为其验收。

## 独立证据索引

- [checks.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/checks.json)
- [findings.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/findings.json)
- [git-evidence.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/git-evidence.json)
- [github-jobs.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/github-jobs.json)
- [github-artifacts.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/github-artifacts.json)
- [ci-job.log](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/ci-job.log)
- [download-verification.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/download-verification.json)
- [independent-probe.cjs](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/independent-probe.cjs)
- [independent-probe.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/independent-probe.json)
- [independent-artifact-check.mjs](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/independent-artifact-check.mjs)
- [independent-artifact-check.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/independent-artifact-check.json)
- [release/dist/public-release.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/release/dist/public-release.json)
- [release/dist/public-browser-results.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/release/dist/public-browser-results.json)
- [release/dist/public-validation.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/release/dist/public-validation.json)
- [release/dist/public-http-validation.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/release/dist/public-http-validation.json)
- [release/dist/public-lab.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-ed91d30-round1/release/dist/public-lab.json)
