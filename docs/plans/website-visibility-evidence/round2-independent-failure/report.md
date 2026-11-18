# [Engineering Mode] v4.2 可恢复隐藏增量 — 第 2 轮正式独立复验

**最终工程结论：FAIL。** V-F01、V-F03 已关闭；V-O01 已通过收窄声明关闭。V-F02 的 CTA 与 fragment 入口已修复，但真实 `/start` 路由仍漏修；同时确认 1 项修复带来的测试契约回归 V-F04。当前剩余 **2 项 P2**。

日期：2026-10-21（Asia/Shanghai）。本结论独立绑定新交付，不沿用首轮 FAIL，也不沿用旧 SEO PASS。**本增量已用完约定的两轮正式验收**；到此记录结论，不自行发起第三轮、修改代码或向实现 chat 派发修复，由主对话决定后续。

## 版本与取证

- 增量原基线：`c17f8c8fe0aa23eb2892377af3be1f6c0bf51a52`
- 首轮失败 HEAD：`ed91d302ff560388315b6cc1149491bb9d5beb95`
- 本轮唯一交验 HEAD：`f70b7c4c75baa0b77646a9fe986866ff67327128`
- 本轮工程 SHA：`44bde4d23ded1cba2d7747e00bc146d5feaf0c96`
- content revision：`49e686770de9ed0f5a924923048c88611956e8d8ae7af93bb73744fa9ba236b1`
- config revision：`a27cd071131edbef9665b638f622bf1a541f90673ee733d077c77f71e2faef27`
- 仓库/分支：`aelf-hzz780/NyxID:codex/website-a-delivery`；本轮新增 5 commits，工程 SHA→HEAD 仅 9 份文档/证据文件，tracked worktree clean。
- [CI run 39582397033](https://github.com/aelf-hzz780/NyxID/actions/runs/39582397033)，job `119162973220`；[artifact 12337521717](https://github.com/aelf-hzz780/NyxID/actions/runs/39582397033/artifacts/12337521717)。
- 独立下载 ZIP SHA256：`1e86ffc0fdc0f38699622de5734d1ca5e49a5bccf8bc9c7c424f5efbdbf9354c`，匹配 GitHub digest；ZIP 12,291,572 bytes，安全解包 798 项。
- manifest `sourceCommit=44bde4d23…`；独立重算 content/config revision 通过。内容与配置摘要和首轮相同是默认输出保持一致，**不表示源码相同**；本轮使用新 sourceCommit、远端 run 与 ZIP 单独绑定。
- 首轮 FAIL 的 report/checks/findings/probe 源码及结果五份归档，与本 chat 独立原件逐字相同。
- 本地仅执行源码读取、内存 TypeScript 转译/纯函数探针、HTML DOM/CommonMark AST 和文件摘要核验；环境 macOS 26.6.2 arm64、Node v23.11.0。远端为 Ubuntu runner、Node 20 配置、Chromium、Nginx/backend fixture，单 worker、默认 off。
- 未修改业务代码，未本地完整构建/浏览器矩阵，未使用 4180/IAB，未启停服务、提交、推送或部署，未向其它 chat 发消息。所有新报告在本独立证据目录；两位子审均只读。

## 首轮发现的关闭判定

| 发现 | 本轮判定 | 独立证据 |
|---|---|---|
| V-F01 · Status 中文首页 | **closed** | 原 `/zh`、`/zh/` 反例均拒绝；17 个合法/非法地址探针全符预期，覆盖编码/重复斜杠/点段/非法编码、API/health，独立状态站根仍通过。 |
| V-F02 · 方案认证邀请码 | **部分修复，仍 FAIL** | 实际 CTA 和 `#/start` 都先入 `/login`，保留顶层 `code` 并覆盖访客外层返回地址为配置目标；实际 `/start` 的 route handler 仍直入工作区，见下。 |
| V-F03 · 反引号 marker | **closed** | 实际门禁通过 24 个单/双引号 key、三种 value 引号、全关/逐项 on 组合；另 4 个前缀、插值、错误配对反例不误报。仍是静态 marker 检测。 |
| V-O01 · 纯 schema 残留 | **closed by claim narrowing** | 新包仍有三个 Zod schema 初始化；源码与交接均明确不承诺 adapter 字节归零，控件/请求实现剔除声明与产物一致。 |

[可重复运行的轻量探针](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/independent-probe.cjs) · [实际结果](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/independent-probe.json)

## 剩余失败

### V-F02 · P2 · 真实 /start 入口仍将邀请码嵌入 return_to

[定位](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/router.tsx:344)。

**预期：**已接受方案的真实 /start、fragment start 与 CTA 使用一致认证入口：code 顶层保留，return_to 绑定配置完整目标。

**实际：**CTA 与 #/start 已修复；真实 /start 的 beforeLoad 未改，仍直接 siteHref(target) 跳工作区。登录保护只生成顶层return_to，LoginPage读code=null。

复现前提：开启 help-desk，配置 target 为 `/assistant?setup=help-desk`（探针同时带 `draft` 参数验证完整目标保留）；仅作为接线 fixture，不声称该方案业务已上线。

1. 未登录访问 /start?t=help-desk&code=NYX-ABCDEF12。
2. 真实 legacyStartRoute.beforeLoad 得到 /assistant?t=help-desk&code=NYX-ABCDEF12&setup=help-desk&draft=hello+world。
3. 受保护入口生成 /login?return_to=<编码后的完整工作区URL>，没有顶层code。
4. Login/Register只读顶层code；AuthFlow邀请预填为空。

**影响：**有效邀请码在嵌套URL中仍存在，但认证无法使用，邀请码必需场景须重新输入。默认off不触发；未修路径属于已确认保留的真实分享入口。

**证据边界：**TypeScript AST提取新源码真实beforeLoad并调用，函数模块使用显式on配置；随后执行精确app-entry/login表达式。未运行真实账号或浏览器。

**建议修复：**让真实/start复用与CTA/fragment相同的方案认证交接，并覆盖其实际route handler；保持未开放方案返回详情和无t通用注册行为。

证据：[independent-probe.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/independent-probe.json), [independent-probe.cjs](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/independent-probe.cjs), [main.tsx](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/main.tsx), [public-paths.ts](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/lib/public-paths.ts), [public-routing.mjs](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/scripts/public-routing.mjs), [2026-10-16-website-scope-cut-v4.2.md](/Users/huangzongzhe/workspace/vibeCoding/nyxid/docs/plans/2026-10-16-website-scope-cut-v4.2.md)

### V-F04 · P2 · on 浏览器分支仍期待修复前的方案 href

[定位](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/e2e-public/public-site.spec.ts:94)。

**预期：**enabled help-desk断言应检查/login以及解码后的return_to等于配置target。

**实际：**测试仍把href与publication.configuration.solutions["help-desk"]直接比较；组件/构建gate已改为/login?return_to=…，所以该分支的正确UI也无法通过。

复现前提：开启 help-desk，配置 target 为 `/assistant?setup=help-desk`（探针同时带 `draft` 参数验证完整目标保留）；仅作为接线 fixture，不声称该方案业务已上线。

1. 使用合法help-desk target，例如 /assistant?setup=help-desk。
2. 浏览器测试第90行导航到无query的/solutions/help-desk。
3. 实际CTA为/login?return_to=%2Fassistant%3Fsetup%3Dhelp-desk。
4. 第94行期望/assistant?setup=help-desk，必然不相等。

**影响：**正式开启该项时会被已过期的browser断言阻断。当前默认off的10项通过结果不覆盖该分支。

**证据边界：**提取原expected表达式，与实际solutionAuthPath/siteHref输出比较；没有把静态确定性失败写成已运行的浏览器失败。

**建议修复：**同步enabled分支的新认证链接契约，解析pathname与return_to逐项断言；保留disabled分支与其余默认回归。

证据：[independent-probe.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/independent-probe.json), [solution-activation.tsx](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/src/features/website-a/solution-activation.tsx), [verify-public-build.mjs](/Users/huangzongzhe/workspace/vibeCoding/nyxid/frontend/scripts/verify-public-build.mjs)

V-F02 的可达性另已核对：`main.tsx` 将真实 `/start` 交给 app-entry；`isPublicPath` 保留该入口；下载包 `application-routes.json`/Nginx 规则将其交给 app shell，没有服务器替代它完成新认证交接。方案未开启时它仍回详情，此问题不会被默认 off 浏览器矩阵发现。

新增 `solution-auth-handoff.test.tsx` 使用真实 siteHref/SiteLink、Login/Register/AuthFlow，并手动调用实际 appRouter validator/auth-layout gate；认证 hooks 与最终跳转采用 mock，页面放入简化 Router。这能证明 CTA 修复，不等于完整真实 app/browser 路由链，也没有覆盖真实 `/start` handler。

## 已通过的实际验证

- 独立 GitHub 日志：**31 files / 241 tests**，含新认证测试 3 项；TypeScript 与完整生产构建通过；**90 页产物门禁、219 项 HTTP、日志隐私、10 项浏览器**通过，0 skipped / flaky / unexpected。
- 新包全部四开关 false、solutions 空；38 个营销 SSR 页面主导航 Blog 关闭且 Footer Blog 保留；18 个 EN/ZH 方案详情原生 disabled；2 个 Contact 保持五字段、mailto 及 SSR disabled。
- 90 份 Markdown 与首轮逐字相同；llms、sitemap、public-routing.conf、application-routes.json 逐字相同；发布路径及 indexable 策略不变，草稿不会因开关恢复自动批准。
- 独立 AST 再核 EN/ZH 首页各 2 个代码块、Agents 各 4 个代码块及批准 tab 内容/链接；450 个 JS 文件按三种引号扫描，无关闭组件 marker。
- 所有 SSR 配置与新 manifest 一致；实际新源码 boundary 再次拒绝同布尔不同方案 target；普通登录/dashboard 默认落点及可信 return_to 校验业务实现无 diff。
- 原 v4.2 计划/review、v4.1 保护和完整重建恢复/回滚流程保持；正式 build 仍不依赖未跟踪视觉切片。

## 未执行与外部依赖

- **not-run：** on 配置完整生产 build/browser、整包 HTML/JS 混装 browser、真实账号/OAuth/MFA、部署与 field CWV。本轮不追加本地重矩阵，也不把轻量函数或 mocked integration 结果写为上述流程通过。
- **blocked（外部验收依赖）：** 真实 Newsletter 确认/送达/退订/滥用控制、公共 Chat 配额/限流、真实 Status 可用性；任一方案完整配置/授权/真实任务；新文章批准。均保持关闭或原发布集，不单独构成本轮 FAIL。
- 新单次 `/security` reduced-motion CI lab：CLS **1.5217294424109988**、LCP **116ms**、73 resources、559985 transfer bytes。仅保留观察，不判性能过关，也不判该增量造成性能回归。

## 检查矩阵

**27 项：18 pass、2 fail、4 not-run、3 blocked。** 每项完整 expected、actual、environment、evidence 与版本绑定保存在 [checks.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/checks.json)。

| ID | 检查 | 状态 | 实际结果 / 边界 |
|---|---|---|---|
| V-C01 | 版本、提交和产物绑定 | **pass** | 五个修复 commits；工程 SHA→HEAD 仅九份文档/证据；真实 ZIP SHA256 匹配，manifest sourceCommit 为 44bde4d23；首轮 FAIL 五份归档与独立原件逐字相同。 |
| V-C02 | 远端 CI 真实性 | **pass** | 31 files / 241 tests；tsc 与完整生产 build；90 pages；219 HTTP checks；privacy gate；10 browser passed，0 skipped/flaky/unexpected。 |
| V-C03 | 默认全部关闭 | **pass** | 38 个营销 SSR 页面无三功能；18 个方案详情原生 disabled；CI 浏览时无同源 POST。 |
| V-C04 | 保留源码与开关独立性 | **pass** | 三组件及 adapter 保留；8 组合 resolver/组件 fixtures 已通过；renderer 覆盖全 off 与三个单项 on；非法/缺验收引用抛错。 |
| V-C05 | Newsletter adapter 行为 | **pass** | 仅接受 pending_confirmation；显示未确认；POST 无凭证，10 秒超时/16 KiB 上限；SSR disabled，失败保留输入；fixture 证据不代表真实订阅。 |
| V-C06 | 公共 NyxBot adapter 行为 | **pass** | 空白回复被拒绝、回复按文本渲染、错误保留问题、取消清理；无 cookie/auth/Agent Key；无 mount 请求。 |
| V-C07 | Status 恢复地址校验 | **pass** | V-F01 关闭：实际 resolver 探针17个合法/非法地址全部符合预期，包含 /zh、/zh/、编码/重复斜杠/点段、非法编码、API/health；独立 Status 域名根及合法子路径保留。 |
| V-C08 | Blog 菜单、Footer 和直达页面 | **pass** | EN/ZH、700/1280 宽度远端默认 off 浏览器通过；38 页 Footer Blog 保留；90 页发布路径/indexable/Markdown 路径一致；llms/sitemap/routing 与首轮逐字相同。 |
| V-C09 | 草稿边界与已批准发布集 | **pass** | publication 输入无 diff；CI publication-footprint 通过；公开路由集合与旧基线相同；新增组件不加载草稿源。 |
| V-C10 | 九个方案默认禁用 | **pass** | EN/ZH 共 18 页独立 DOM 检查通过；辅助提示保留，通用登录和联系入口仍可用。 |
| V-C11 | 逐方案配置和 legacy gate | **pass** | 九个逐项组件/legacy resolver fixtures 通过；已修复 CTA 与 fragment start 的认证入口；真实路径 /start 的已接受分支仍漏修，单列 V-C12。单项配置独立性未回退。 |
| V-C12 | 恢复方案入口的认证上下文 | **fail** | V-F02 部分修复、仍 fail：真实 CTA 与 #/start 已保留顶层 code 和配置 return_to；但 router.tsx:344 的 /start 仍直接 siteHref(target)，实际 handler 探针后进入登录时顶层 code=null。 |
| V-C13 | 完整配置与 revision 一致 | **pass** | 独立重算新包 config/content hash，所有 SSR 配置属性对齐新 manifest；源实现未回退；在新源码上再次调用实际 boundary 函数，拒绝同布尔不同 solution target。 |
| V-C14 | 已批准 tab 全文与 Markdown | **pass** | 新包90份 Markdown 与首轮逐字一致；EN/ZH 首页各2 code blocks、Agents各4 code blocks；独立 AST markers 和链接通过。 |
| V-C15 | 当前默认包的可操作组件排除 | **pass** | 新包450个 JS 文件按三种引号扫描无关闭组件 marker；实际 optional chunk 控件导出为 null，无相关请求实现；纯 Zod schema 残留已在源码与交接准确披露，V-O01关闭。 |
| V-C16 | bundle 防残留门禁 | **pass** | V-F03关闭：实际gate的24个key/value引号与单项on组合通过；4个前缀/插值/不配对定界符反例不误报。单引号key及不配对引号由本轮独立探针补验。仍仅为完整静态marker检测，不声称任意JS语义分析。 |
| V-C17 | 文档与恢复/回滚步骤 | **pass** | 既有 v4.2 计划/review 和完整恢复/回滚流程保留；修复交接明确旧FAIL与新CI边界，V-O01声明收窄；本轮发现的真实/start漏项及on浏览器断言另列失败，不以文档声称修复覆盖。 |
| V-C18 | 既有默认路径回归 | **pass** | 新CI中既有 mailto/topic=bundle/Discord、默认dashboard/可信return_to、query/retry/menu回归通过；认证业务核心无diff。独立 Contact 字段/SSR检查通过；真实/start的遗漏单列V-C12。 |
| V-C19 | 提交范围与构建独立性 | **pass** | 五个新增commits；tracked tree clean；v4.1/publication/正式build依赖链无新增diff；正式构建仍不依赖未跟踪视觉原型。首轮原始FAIL归档保真。 |
| V-C20 | on 配置完整生产 build/browser | **not-run** | 完整生产构建和browser仍只覆盖默认off；on fixtures不等于on完整包。本轮另静态证实 enabled help-desk 浏览器分支的期望已过期，见V-C27；没有执行on浏览器运行。 |
| V-C21 | 整包 HTML/JS 混装浏览器场景 | **not-run** | 新源码实际boundary函数仍拒绝不同target，CI覆盖不同endpoint；未执行整包替换JS的browser测试，不以纯函数证据代替E2E。 |
| V-C22 | 真实 Newsletter / 公共 Chat / Status | **blocked** | 未提供真实端点/状态地址及完整验收记录；仅 fixtures；保持关闭。非本轮代码失败原因。 |
| V-C23 | 方案完整真实任务 | **blocked** | 无九方案任一真实端到端任务验收证据；TG 已连接/登录成功不算方案完成。邀请码缺陷本身仍需修复，见 V-F02。 |
| V-C24 | 部署与真实账号/OAuth/MFA | **not-run** | 未部署、未连接真实账号/OAuth/MFA会话，未启停服务、未访问4180或IAB；本地验收地址交由主对话按约定另行处理。 |
| V-C25 | 真实性能及 field CWV | **not-run** | 仅 /security 单次 reduced-motion CI lab：CLS 1.5217294424109988，LCP 116ms，73 resources；不判性能通过，也不判本增量回归，field CWV未测。 |
| V-C26 | 候选文章发布批准 | **blocked** | 本轮未批准任何候选文章；既有批准集及索引策略保留。 |
| V-C27 | 开启方案后的浏览器断言契约 | **fail** | V-F04：public-site.spec.ts:94 仍要求原始 /assistant 或 /dashboard target；实际CTA pathname固定/login，合法配置下二者不能相等。默认off CI走else，未覆盖此分支。 |

## 独立证据索引

- [checks.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/checks.json)
- [findings.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/findings.json)
- [git-evidence.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/git-evidence.json)
- [github-artifacts.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/github-artifacts.json)
- [github-jobs.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/github-jobs.json)
- [ci-job.log](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/ci-job.log)
- [ci-verified-excerpts.txt](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/ci-verified-excerpts.txt)
- [download-verification.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/download-verification.json)
- [independent-probe.cjs](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/independent-probe.cjs)
- [independent-probe.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/independent-probe.json)
- [independent-artifact-check.mjs](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/independent-artifact-check.mjs)
- [independent-artifact-check.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/independent-artifact-check.json)
- [unchanged-public-surfaces.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/unchanged-public-surfaces.json)
- [release/dist/public-release.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/release/dist/public-release.json)
- [release/dist/public-browser-results.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/release/dist/public-browser-results.json)
- [release/dist/public-validation.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/release/dist/public-validation.json)
- [release/dist/public-http-validation.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/release/dist/public-http-validation.json)
- [release/dist/public-lab.json](/Users/huangzongzhe/.codex/visualizations/2026/10/15/01a13edf-2b31-75c1-b1ca-824d707f02ff/website-visibility-acceptance-f70b7c4-round2/release/dist/public-lab.json)
