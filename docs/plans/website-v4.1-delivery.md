# 官网 A + v4.1 交付 / Website A + v4.1 delivery

> 最新全站验证见 [2026-10-16逐页自查报告](website-all-pages-audit-2026-10-16.md)：25条路由×中英实际组件矩阵、59次本地页面/Auth/Docs HTTP200、线上公开认证配置一致；15文件199测试单worker串行通过，隔离生成68输出一致。NAV-01与Legacy Auth查询修复已纳入。此结果不等于图形视觉/真实账号OAuth或邮件投递验收；订阅与背景水印仍未完成。
> Current page/link/auth-entry evidence is in the all-pages audit.199 serial tests and68-file regeneration passed; physical/graphical/real-account/client delivery and the open newsletter/branding items remain outside that conclusion.


当前状态：Contact 邮箱客户端交接及公共按钮渐变修复已完成定向验证（见文末最新增量），等待外部视觉/客户端验收；**原场景品牌水印尚未解决，整体不能判定完成**。组织 PR 由负责人手动创建。本文覆盖先前 A-only 文档里的“不接 B”旧范围。

Status: implementation and theme fix verified locally, pending independent acceptance. **The original scene still carries branding; overall delivery is incomplete.** The owner creates the organization PR manually. This supersedes the historical A-only scope.

## 目标、基线与边界 / Goal, baseline and boundaries

保留已通过 A 的双语内容、25 条公开路由、真实 AuthFlow、完整查询参数、9 秒人群轮换、原生五工作演示和筛选，按已批准 V01–V06 提升表现。基线 `5de5ff19f8347b09604221ce260089e1b5a1c6e1`，其运行代码为 A 验收版本 `1ac8a28675603279499fb1205790ca4bd67c4fa0`。未改后台、鉴权、权限、权益或计费，不新增 mock 注册/联系成功。

Preserve accepted A content, routes, real authentication, full query propagation and native interactions while integrating approved V01–V06. No backend, auth, entitlement or billing changes. Previous A acceptance is historical evidence only.

批准输入仍在未跟踪的 `docs/plans/website-review/`，未修改或提交。产品原稿 SHA-256 `5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33`；候选 HTML SHA-256 `a620057104bf31b4e55be85244ea6ba29146e7ca7ecaa549c988e499aafc4d47`。生成文件编译为正式 React，不在运行时加载评审 HTML 或执行其脚本。

Approved review inputs remain untouched and untracked. Their hashes identify the design references; production compiles committed React source without review HTML, global DOM rewiring or extra React roots.

## 实现映射 / Implementation mapping

| Item | Native implementation and limits |
| --- | --- |
| V01 | Original UnicornStudio v2.1.4 scene `sqyz3r4WjVjdLal6IHvt`, delayed load, 30fps/dpi1, approved opacity; actual GradientText, 9s purple gradient. Chinese “AI，超越聊天。”; original audience interaction preserved. Branding remains unresolved. |
| V02 | Original 24 static cards unchanged: desktop 6 columns, 390px 2 columns, gap10, natural height58. No DriftWall, clones, masks or new controls. |
| V03 | Actual SpecularButton shader decorates existing native anchors; autoAnimate=false/proximity90. Offscreen/background/reduced motion removes decoration. StrictMode setup owns a fresh canvas; partial GPU failures retain readable links. |
| V04 | Actual SpotlightCard only on existing full-card links, including focus. Static tools, prices, permissions and form controls excluded. Hidden filtered cards release decoration. |
| V05 | Actual FadeContent GSAP timeline: 600ms, initialOpacity .72, no blur, once. Only Home doors, How-it-works paths, Agents resources and Blog covers; original demos/code/prices/permission content excluded. |
| V06 | Approved restrained borders/shadows on demos, native film and code; original single price example and tabular numbers preserved. |

展示层适配在 `visuals/`，生成器负责准确放置原生 wrapper。共享活动 gate 只管理新增动效；observer、visibility/media listeners、timer、GSAP timeline、GPU 资源有明确销毁。网络/渲染失败保留内容和入口；固定诊断标签不带账号或凭据。

Adapters live in `visuals/`; generation places native wrappers precisely. Activity gates manage only new effects and release observers, listeners, timers, GSAP timelines and GPU resources. Failures preserve content/navigation and emit only non-sensitive diagnostics.

React Bits 来自固定 commit `496968c4117e21278f13917201b7d615bc5e7450`，完整 **MIT + Commons Clause** 和上游输入哈希、适配理由见 [PROVENANCE](../../frontend/src/features/website-a/visuals/react-bits/PROVENANCE.md)。Fade 使用正式锁定依赖 GSAP 3.15.0，没有照搬原型全局 wire/MutationObserver/多 root。

Pinned React Bits sources, the complete license and native adapter explanations are retained. GSAP 3.15.0 is a locked production dependency; prototype-wide mutation wiring is excluded.

## 主题修复 / Theme correction

公开官网固定为独立 dark，根节点、标题渐变与卡片一致，不读取或改写 `nyxid.theme`，不改变系统/应用主题和登录、注册、后台设置。保留浅色样式作为已有资源，不新增切换控件。旧浅色截图仅作历史探索，不能当成最终批准主题。

Public marketing pages independently render dark. Root, title and card decoration agree without reading/writing app preferences. Application themes remain unchanged. Existing light CSS is retained; no theme control is added. Earlier light screenshots are historical exploration only.

## 验证与限制 / Verification and limits

- 16 个测试文件 / **125 tests passed**：官网、router/public-path、Login/Register/return URL、Docs、应用 theme hook/store。新主题测试先证明旧实现 system+light/显式light 两项失败，再证明三种模式均修复；重挂载模拟刷新和路由返回，并核对存储与应用主题不变。
- 完整 `npm run build`、官网 ESLint、`git diff --check` 成功；构建仍报告 Tailwind text-destructive 选择器和大 chunk 警告，未宣称无警告。
- 生成器 68 个输出逐字节复现，LICENSE 与输入完全相同，见 [generation.json](website-v4.1-evidence/generation.json)。构建官网共享 elements chunk 为 88.50kB / gzip34.58kB（含 GSAP）；没有用户 p75 CWV 或物理设备性能结论。
- 真实内置浏览器：桌面1280×720、中英390×844；完整24项几何；移动中英菜单登录 Enter 与 code、嵌套return_to、UTM、lang 保留；核心页面双语及 Security/Integrations/Login 排除对照。八组 NyxBot 侧栏在1280×900均无横溢出。
- 实际 shader 光强从 idle .450 到接近时 .907；渐变随时间变化；Spotlight focus 光层 .1 且原 anchor focus outline 保留；Fade .8004 →1，离开返回仍1；Hero离屏paused、标题装饰卸载、CTA画布0，返回恢复同场景。
- GPU/脚本失败、动态 reduced motion、document.visibility、延迟场景卸载等用组件模拟测试验证。浏览器标签切换未使 document.hidden=true，不能冒充真实后台停用证据。未执行真实 OS 媒体设置、真实 OAuth/MFA、物理设备、Firefox/WebKit 或生产指标。

Tests, production build, lint and deterministic generation passed. Browser observations and simulated lifecycle evidence are separated in [evidence README](website-v4.1-evidence/README.md) and [browser audit](website-v4.1-evidence/browser-audit.json). Exact acceptance remains pending; no blanket “zero errors” or performance guarantee is claimed.

## 未解决项与下一步 / Open item and next step

首屏显示 “Made with unicorn.studio”。仓库范围查找只发现同一旧场景与评审引用，没有可用的已授权无品牌 scene/export。来源对话实际核验线上 DOM 与截图：线上使用同 Project ID / SDK，品牌图仍存在，只被旧 hero 的 120px 底部渐变层覆盖，因此线上看不见不代表另有无水印场景或授权变化。旧 hero 的遮挡水印层没有移植；本次没有删除品牌 DOM、隐藏品牌 CSS、改写第三方 SDK 或付费。账户/订阅授权未知。

The original scene displays “Made with unicorn.studio”. Repository search found no alternative authorized watermark-free scene/export. The source chat inspected production DOM/screenshots: the same scene/runtime contains the brand image, covered by the legacy 120px bottom gradient. This explains the apparent production difference without inferring an account downgrade or a different asset. The old cover layer is not carried forward. No branding removal, SDK modification or purchase was performed; account/license state is unknown.

可行完成路径：取得原场景正式无水印发布资源并通过现有环境配置接入，或负责人确认改用已有 React Bits / 自有背景后实施。静态降级只负责故障可用性，不能替代“保留原背景”验收。背景决策不阻止其余实现 Commit 和独立检查，但阻止整体通过。

Resolution requires an authorized watermark-free publication of the original scene, or owner approval for a React Bits/owned replacement. Static fallback is resilience rather than fulfillment of the approved background. Other implementation work can be committed/reviewed while this blocks overall acceptance.

## 本地运行、配置与回滚 / Run, configuration and rollback

依赖安装使用 `cd frontend && npm ci`；当前单实例预览由根线程管理，带生产后端代理的准确启动命令见下文“当前单实例预览”，避免重复启动。预览 `http://127.0.0.1:4180/?lang=zh`。生产构建不依赖未跟踪的评审源文件。

The local preview is retained on port4180. Production builds require only committed source and locked dependencies.

`VITE_WEBSITE_BACKGROUND_RUNTIME_URL` / `VITE_WEBSITE_BACKGROUND_PROJECT_ID` 允许配置已授权资源；`VITE_WEBSITE_VISUALS_ENABLED=false` 可重建为静态增强回退。需要撤回本次代码时，通过正常 revert Commit 回到上述 A 基线，不修改应用主题存储，不部署或强推。

The background URL/project can be configured for authorized resources. A build-time visual flag provides static resilience. Use a normal revert to restore accepted A; no preference reset, force push, merge or deployment is required.

## 增量配色修正 / Brand-gradient correction

负责人追加：首页中英文强调词与 How-it-works 的“安全地”统一为 `#A672FB → #5E00F5`，移除浅白/灰紫色 stop。保留9秒 GradientText，静态/reduced-motion/离屏回退统一复用 `var(--vivid)`；未改参照页、文案、字号或布局。

The Home emphasis now uses the reference page's two brand-purple endpoints while retaining the nine-second GradientText animation. Static fallbacks reuse the original vivid token. The reference page, content and layout are unchanged.

增量验证：13项相关测试、改动 ESLint、68个输出生成复现通过；真实浏览器核对中英动画、中文参照页和离屏静态 computed gradient 一致，返回后动画恢复。见 `website-v4.1-evidence/color-audit.json` 与 `color-*.jpg/txt/json`。整体125项/完整构建记录属于前一实现 commit；本纯配色修改没有重跑全量。水印阻塞不变，独立验收按新 commit 增量复验。

Incremental checks passed:13 related tests, scoped lint and68-file generation equality. Browser evidence records both animated languages, the reference and static fallback, plus animation recovery. Earlier full-build/125-test evidence belongs to the previous implementation commit; no full rerun was needed for this palette-only change. Branding remains unresolved.

## Contact 恢复阶段（历史记录，收件方式已在下文接通） / Contact restoration history

静态对比确认生成器此前将批准表单和 NyxBot 问答整块替换为 Discord，导致功能界面丢失。现恢复原双列结构、五字段（公司选填）、五类主题和 Bundle 查询预选，正确 label 与原必填/邮箱校验；以原生 React、useAppForm 和 Zod 实现。右侧 FAQ 保留四个快捷问题、输入框、本地规则答复及原 Discord 卡；Bundle 问答可同步选中主题。演示明确不调用真实 AI，并保留真实登录入口。

Static comparison identified an overbroad Discord substitution in the generator. The restored native interface includes five labelled fields, optional company, five topics, Bundle preselection, original validation, scripted FAQ and the existing Discord card. FAQ responses are local, bounded and timer-free; real NyxBot remains behind sign-in.

接收邮箱/接口待负责人提供。此阶段只做本地校验，明确提示未发送、不保存；不生成伪成功、不猜收件人、不绕过匿名额度。尚不构成 Contact 完整交付。已编写相关回归测试，但按 CPU 降载要求尚未执行测试、构建、生成器复现或浏览器验证；两个生成页暂为按新生成规则静态同步，恢复验证后须核对逐字节复现。未提交用户系统日志，不推断网站 WebGL 已被证明是 Codex 退出根因。

Delivery awaits an owner-provided recipient or endpoint. Entries are validated locally and explicitly remain unsent; no fake success, guessed recipient or anonymous AI is introduced. Contact is incomplete. Tests are authored but unexecuted under the CPU constraint; build, generation equality and browser checks are deferred. The two generated pages were aligned statically and require reproduction verification when testing resumes. System diagnostics are excluded from repository artifacts.

### Contact 样式静态核对 / Contact style mapping

按负责人“先把样式处理了”完成样式代码：原 `.wrap.contact` 的 `1.15fr/.85fr` 双列与20px间距、`.card` 的22px内边距/16px圆角、`.contact-form form` 的16px行距与22px顶部留白、`.f2` 的两列/14px间距、44px输入控件、五行留言及46px全宽主按钮均匹配恢复后的原生DOM。原 `.contact-side`、`.bot`、`.bot-h`、240px `.bot-msgs`、快捷chips与Discord样式继续生效；原960px media rule将左右两栏及字段分组均堆叠为单列。

The restored DOM matches the original two-column, card, form, input, button, FAQ and Discord selectors in `website-a.css`, including its960px single-column breakpoint. No replacement design or content layout is introduced.

补充 `contact.css` 仅限制Contact范围：网格子项/输入允许收缩，问答输入与44px发送按钮共用行宽；长问题换行、消息不被挤缩；演示说明增加12px留白并显示真实登录链接；无效字段使用已有错误色。没有改全站主题/动画或原型字体token。接收渠道待确认；样式已提交代码，尚未做浏览器/窄屏实测、构建或测试，不能据静态匹配宣称像素验收通过。

Contact-scoped CSS adds shrink safety, long-question wrapping, stable message sizing, readable demo disclosure/sign-in and existing error-color feedback. Recipient integration and runtime verification remain pending; static selector mapping does not establish visual or responsive acceptance.

## 当前增量：按钮渐变与联系邮箱 / Current increment: button paint and contact email

负责人已指定 `contact@chrono-ai.fun`，表单采用邮件客户端交接，无后台公开发信 API。`VITE_WEBSITE_CONTACT_EMAIL` 为单一 build-time 配置；缺省为指定邮箱，非法单邮箱配置回退并记录固定无敏感信息警告。页面有可访问的邮箱链接；有效表单点击“打开邮件客户端”后只请求打开已填收件人/固定主题/正文的 mailto，正文包含姓名、回复邮箱、可选公司、主题及留言。客户端须由使用者确认发送；页面不宣称已发送、也不清空草稿。异常分支和未打开客户端时仍可使用邮箱链接，FAQ 套餐指引同步更新。此前“渠道接入中”状态已替换。

The owner-provided mailbox is configured centrally through `VITE_WEBSITE_CONTACT_EMAIL`, defaulting to the specified address. The form composes a mailto draft only after validation; it includes all contact details in the encoded body and fixed topic labels in the subject. Users review/send in their email client. The page retains the draft and direct address and never reports successful delivery. No backend endpoint, external request, paid email service or anonymous AI is added.

编码边界：收件人只允许单邮箱，原始 CRLF 拒绝；表单先经 Zod 校验。用户姓名/公司等只进正文且单行化，留言换行规范化为 CRLF 后整段 encodeURIComponent。只有固定 subject/body 两个参数，无 cc/bcc/Reply-To 头；特殊字符、中文、emoji、+、&、百分号与头注入形状文本不改变收件人或参数集合。

Recipient validation rejects raw CRLF and multiple recipients. User values stay in the encoded body; only fixed topic labels enter the subject. Body line breaks are normalized and URI-encoded, preventing query/header escape. There is no automatic send or proof that an OS email app is installed.

公共主按钮 `.btn.p` 和较晚的 `.mock .btn.p/.btn.p:hover` 均改为 `background:var(--vivid) border-box no-repeat`，让渐变从边框范围完整绘制，避免透明1px边框处重复左端亮色。生成器使用同一小型 PostCSS 转换防止重新生成回退。普通次按钮真实边框、focus outline、原渐变、Specular 没有移除；后续实色 `.band .btn` 设计不变。该解释由 CSS 默认绘制模型支持，实际亮边是否消失仍由外部浏览器验收，未用 Codex 内置浏览器复现。

Both shared and mock primary backgrounds now paint once across the border box. Generation reuses the same transformation. Secondary borders, keyboard focus and approved animation remain. Solid band-button backgrounds retain their original design; visual confirmation remains pending.

本轮按降载要求仅串行执行两个相关文件、单worker：Contact/helper 9项测试通过（约1.10s），包含中英有效/无效表单、mailto内容/编码、CRLF边界和模拟客户端启动失败时保留草稿/邮箱入口。相关5文件 ESLint、生成器语法和 Git whitespace 通过。另做一次性 Node CSS 静态核对（2项通过）；没有提交镜像 CSS 测试。未启动 IAB/CUA、真实邮件客户端、服务、完整生成器、完整类型构建或全量测试，未重启4180。见 `contact-mailto-checks.json` 与相关日志。旧125项/完整build证据不冒充当前HEAD验证；整体水印阻塞仍存在。

Incremental evidence covers nine targeted behavior/encoding tests in a single serial worker, scoped lint, generator syntax and one-time CSS checks. No browser, OS client, full generation/build/suite or service startup ran. Existing production preview remains managed by the source chat. Overall acceptance remains blocked by branding and unperformed visual/OS checks.

## 全站审计增量：Legacy Auth 查询 / Site audit increment: legacy auth queries

只读全站调查发现 legacyDestination 对真实 `#/login` / `#/signup` 丢弃 fragment query；本次修正为合并外层 search 与目标 fragment query，目标同名参数优先，保留 code、嵌套 return_to、lang、UTM 等。只在首个 `?` 切分，避免截断参数值里的合法问号。实际跳转路径仍是固定本地 `/login` / `/register`，其它公开路径仍须经过原白名单；不放宽协议相对路径、编码绕过或外部路径。

Legacy real-auth fragments now retain the outer and fragment query contexts, with destination parameters winning conflicts. Splitting only the first query delimiter preserves question marks inside values. Destinations remain fixed local auth paths and the existing public-route allowlist is unchanged.

兼容策略：原型 `#/start` / `#/done` 不再模拟配置成功；只把原外层 search 带到真实注册，整段丢弃 mock fragment query（包括 t），不把原型方案选择当作真实凭据/开通状态。回归覆盖合并、优先级、嵌套参数、问号及 mock 丢弃；原外部路径防护用例保留。根线程已统一串行验证通过；详见 [逐页自查报告](website-all-pages-audit-2026-10-16.md)。

Mock Start/Done fragments continue to enter real registration using only the outer query; their fragment state is intentionally discarded. Regression cases cover real-auth merge/precedence, nested query values and discarded mock flow fields. The source chat's coordinated serial run passed; see the linked all-pages audit.

核心 AuthFlow 不修改：登录/注册面板跨越时仅保留 code/return_to，与 upstream/main 相同，按“登录保持线上”保留此既有行为；不能把官网入口的完整 query 保留扩大宣称为整个 AuthFlow 都保留 lang/UTM。可信 return_to 和 MFA/Provider/邀请策略继续由原实现负责。

Core AuthFlow remains unchanged to match production: its panel switch retains code/return_to only. Website-link propagation does not imply arbitrary query preservation through all authentication steps. Existing trusted-return, MFA and live provider/invitation rules remain authoritative.

### 当前单实例预览 / Current single-instance preview

由根线程管理现有4180服务，本交付对话不重启或另起。根线程已确认公开配置请求200并逐字段与线上一致（github/google/apple；invite_code_required=false；email_auth_enabled=false），这仅是公开配置核验，不证明账号登录/OAuth/MFA完成。当前配置启动方式：

The source chat manages the single existing4180 service and reports parity of public authentication configuration. This is not real-account authentication evidence. Current command:

```sh
cd frontend
BACKEND_URL=https://nyx-api.chrono-ai.fun FRONTEND_URL=https://nyx.chrono-ai.fun NODE_USE_ENV_PROXY=1 npm run dev -- --host 127.0.0.1 --port 4180 --strictPort
```

默认未配置 backend 的本地命令可能使 public/config 代理失败；不能以这种502推断线上禁用登录，也不能以当前200推断所有鉴权流程通过。本交付对话未运行浏览器/测试/构建或更改服务；根线程统一验证与 HTTP/配置检查结果见逐页自查报告。独立 page-matrix.test.tsx 由根线程编写并与本轮修复一并交付。

An unconfigured local backend can produce proxy502 and is not evidence of disabled production login. A200 public configuration is not full auth verification. This delivery chat ran no browser/tests/build or service changes. The source chat completed coordinated tests and HTTP/config checks; its independent page matrix is included with these fixes.

### NAV-01 当前栏目恢复 / Current navigation restoration

独立静态审查发现原HTML的 `.nl.on` 高亮在生成时遗漏；此前只有精确路由 aria-current，没有实际选中样式，详情页也无所属栏目。本次以共享 `websiteNavProps` 同时恢复 class 与可访问状态：顶级页为 page，方案/博客等子路径归属对应栏目为 location；使用完整 `/` 段边界，`/solutions-old`、`/blogger` 不误选。中英文Header与生成器同步，href/query、移动登录和原CSS不改变。

The shared navigation adapter restores both selected styling and accessible current state. Exact pages use aria-current=page; descendants mark their owning section with location. Segment boundaries prevent prefix collisions. Both generated headers and the generator agree without changing destinations, query context or mobile login.

已在既有 generated-contract 测试中补7顶级栏目、2类详情归属及无匹配/相似前缀场景，保留原链接参数测试。根线程已统一串行验证并纳入独立页面矩阵：15文件199项通过，生成器隔离复现68个输出一致；无新增图形浏览器检查。

New header regression sources cover every top-level column, solution/blog descendants and unrelated/prefix-like paths. The coordinated source-chat run passed199 tests across15 files and isolated regeneration matched68 outputs. This delivery chat did not repeat those checks or open a browser.
