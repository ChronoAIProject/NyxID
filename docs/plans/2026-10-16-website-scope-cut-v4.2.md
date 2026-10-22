# NyxID 官网裁剪方案 v4.2 / Website scope revision

日期：2026-10-16；**2026-10-21 更新：本期隐藏、保留实现、后续可配置开放**。状态：范围修订已确认，当前默认外观保持关闭，工程验证见独立交接。本文独立于 v4.1 保存；只调整产品负责人已确认的六项范围，沿用其余已批准的视觉与内容。

Date: 2026-10-16; **updated 2026-10-21: hide for this release, retain implementation, and enable individually through configuration after acceptance.** Defaults remain off; see the separate engineering handoff for verification. This separate document revises six agreed items and retains the remaining approved content and visual direction.

**[查看 2026-10-21 默认隐藏视觉对照](website-scope-v4.2/visibility-review.html)** · **[查看 v4.1 → v4.2 视觉对照](website-scope-v4.2/review.html)** · [v4.1 原方案](2026-10-14-website-upgrade-proposal.md) · [v4.1 视觉候选](website-review/nyxid-ste-preivew-new.html?lang=zh&clean=1#/)

## 1. 依据、目标与边界 / Basis, goal and boundaries

本次“之前”固定指 **v4.1 视觉候选及其方案**，不是老官网，也不是最初的 A 原型。代码现状另列，防止把已经处理的事项算成新增交付。讨论依据为[《梳理官网动态功能范围》](codex://threads/01a13e83-15d6-7873-8786-ffd2d04cde0a)；其中历史建议以本轮产品负责人结论为准，尤其是 dashboard 和 Use case 的处理。

“Before” consistently means the **v4.1 visual candidate and proposal**. Implementation status is listed separately. The latest product decisions override earlier recommendations in the linked discussion, particularly dashboard routing and use-case availability.

目标：让官网展示、入口状态与实际可交付能力一致。保留商户理解场景、阅读文章、注册和联系团队的入口；将 CMS、订阅运营和公共客服等需要独立服务的工作后移。官网为“商户或施工队理解方案 → 配置 → 完成实际任务”提供入口；不能把页面展示或登录成功等同于商户方案已交付。

Goal: align website promises and entry points with deliverable capabilities. Keep discovery, reading, registration and contact available while deferring separate operational services. The website supports merchant or installer onboarding; rendering pages or signing in does not prove a merchant workflow works.

非目标：本轮不部署 CMS、不接真实 Newsletter/公共 NyxBot 后端、不建设 feature-flag 管理后台、不改默认登录落点，也不声称任一 Use case 已打通。保留首页轮换及 hover/focus、原尺寸静态工具墙、紫色视觉与既定动效；不借本次裁剪重写其它营销内容。背景水印问题仍是独立未决项，六项裁剪不代表该问题已解决。

Non-goals: CMS deployment, live newsletter/chat integration, a feature-flag administration service, changing the default post-login destination, or declaring any use case operational. Existing approved visuals and interactions remain. The unresolved background branding issue remains separate.

## 2. 六项前后对比 / Six scope decisions

| 编号 | v4.1：之前 | v4.2：本次裁剪 | 原因与收益 | 当前代码与裁剪的差距 |
|---|---|---|---|---|
| C01 Blog | 有列表、分类、精选、文章详情和相关阅读；没有明确本期正式发布方式 | **主导航及移动菜单的 Blog 默认隐藏，保留独立恢复开关；完整阅读体验与审核后静态发布保留**；通过仓库 PR 更新，CMS 成熟后再接 | 内容先可阅读；不把运营后台建设绑进官网交付 | 新版 6 篇原型文章已生成静态页面；旧 Blog 仍有 mock/预览路径，需要确定正式文章集和草稿边界 |
| C02 订阅 | Blog 底部有邮箱输入、Subscribe、每周更新及退订承诺，并列 Discord | **订阅本期隐藏，保留组件与 adapter，只展示 Discord；通过独立配置开放**；不保留“订阅即将开放”占位 | 不承诺尚无收集、发送、退订链路的能力，保留现有交流渠道 | 本轮保留可恢复表单与真实响应 adapter；默认发布输出不包含订阅控件或承诺 |
| C03 Contact | 左侧表单，右侧 NyxBot 问答演示、问题快捷项、输入框及 Discord | **NyxBot 聊天卡本期隐藏，保留可维护组件与 adapter**；保留邮件表单、`topic=bundle` 预选及 Discord | 避免把脚本演示当成可用的公共客服；联系团队仍可完成 | 原型脚本回复不接回；新组件仅接受配置后真实服务返回的文本，邮箱保持 `contact@chrono-ai.fun` |
| C04 Footer | 有 Status / 服务状态，实际指向首页占位 | **隐藏 Status 入口**，其余有效链接保留，页脚自然收拢 | 不把普通页面误称为服务状态页 | 默认关闭；保留 FooterStatus 组件，真实公共状态地址验收后单项配置开放 |
| C05 Dashboard | v4.1 视觉原型不表达真实登录会话；既有交付约束是沿用线上认证 | **保持现有默认 dashboard 逻辑**；有效 `return_to` 仍按原逻辑处理；未来再切到登录后的 NyxBot | 控制认证回归，避免在官网裁剪中改变产品入口 | 当前默认行为已符合；重点是回归，不是新跳转开发 |
| C06 Use case | 9 个详情页“使用此方案”均为可点链接，进入原型 `#/start?t=…` | **默认灰色禁用；一个方案完整调通并验收后，只开放这一个** | 用户不会被可点按钮带入不完整配置；支持逐个积累可交付方案 | 当前 start 被映射至注册，方案上下文未完整交接；仅能注册不算打通 |

English summary: keep Blog reading with reviewed static publishing; hide newsletter signup and Contact chat for this release while retaining their components/adapters; keep Discord and default-off Footer Status; retain dashboard and valid `return_to` behavior; disable each use-case activation until its complete workflow passes acceptance.

## 3. 页面与交互的具体裁剪 / Page and interaction changes

### Blog：保留文章，收敛发布方式 / Keep reading, simplify publishing

精选、分类筛选、文章卡、正文、相关阅读、文章内“浏览方案”和 Discord 均保留；默认发布时不渲染邮箱订阅控件及周更/退订承诺；源码中的订阅组件、校验及 adapter 保留。Discord 使用既有名称、图标和已配置地址，不增加收集邮箱或等待名单。与计费有关的“订阅费”说明不属于 Newsletter，不改变。

Keep featured posts, categories, cards, articles, related reading, solution browsing and Discord. Exclude newsletter signup and unverified delivery promises from default public output while retaining source components, validation and adapters. Billing references to subscriptions are unrelated and remain unchanged.

2026-10-21 最新补充：`VITE_WEBSITE_BLOG_MENU_ENABLED=false` 只控制顶部主导航与移动菜单，恢复时设为 `true` 并重建。Footer Blog 不改；`/blog`、`/zh/blog` 与已审核文章直接访问、既有 Markdown/llms、sitemap 和索引规则保持原策略，不因菜单隐藏而下线、404 或新增 noindex。未审核文章不自动开放。

The Blog menu switch controls only desktop and mobile primary navigation. Footer Blog, direct access and existing SEO/AI publication rules remain unchanged. Hiding the menu neither withdraws Blog nor approves pending articles.

静态不是“只有壳”：本期必须有审核通过的真实正文、标题、摘要、作者/日期、分类、稳定 slug 和现有中英文呈现。初始候选为原型 6 篇，但是否作为正式发布文章由内容负责人确认；原型示例不得包装为已验证商户案例。文章通过“内容审核 → PR → 预览 → 发布 → 必要时撤回”维护，暂不提供运营自助后台、定时发布或 AI 自动发布。

Static publishing includes reviewed article content and metadata, stable URLs and the existing bilingual presentation. The six prototype articles are candidates pending editorial approval; illustrative cases must remain clearly identified. Updates follow review, PR, preview, publication and rollback, without a self-service CMS or automated publishing service.

旧文章 URL 中已公开且审核通过的内容保持兼容；未发布、未知或撤回的文章返回明确未找到状态。旧 `mock-data` 中的 draft、in_review 不可随公共 bundle 发布；`/preview/$id` 不得当作安全草稿预览。既有公开文章与草稿要分开清点，不能为“所有 URL 都能打开”而发布草稿。CMS 文档是未来接入依据，不是已上线证明。

Preserve approved public legacy URLs, but do not publish drafts or in-review content in the public bundle. Unknown, withdrawn or unpublished articles should show a clear not-found state. The legacy preview route is not a secure draft-preview service. Existing CMS documentation describes a future integration, not an operational deployment.

### Contact：邮件表单 + Discord / Email form and Discord

保留五个字段及校验，保留 `topic=bundle` 预选。左侧表单维持主要宽度；右侧默认只展示既有 Discord 卡，NyxBot 卡保留源码并由独立开关控制，顶端对齐，不留下聊天卡空框、输入框或“即将开放”占位；移动端依次为表单、Discord。整体间距、边框和标题层级沿用 v4.1。

Keep the five fields, validation and bundle topic preselection. Retain the main form column and align the remaining Discord card at its top; exclude the chat card from default rendering without an empty frame or replacement placeholder; retain its implementation behind an independent switch. On mobile, place Discord after the form.

沿用已确认邮箱 `contact@chrono-ai.fun`。当前表单是 **mailto 打开邮件客户端**，按钮和说明继续明确“打开邮件客户端，确认后发送”；不能恢复 v4.1 演示中的假“已发送”。本次不增加邮件接收 API 或工单系统。NyxBot 产品介绍页和已登录个人 NyxBot 不在本期隐藏范围。

Keep the approved mailbox and the existing **mailto handoff**. The UI must explain that the visitor reviews and sends the email in their email app; never restore the prototype’s simulated success message. This change does not add a server submission endpoint or remove the public NyxBot product page or authenticated personal assistant.

### Use case：浏览可用，开通受控 / Browse now, activate after acceptance

| 入口或状态 | 本次行为 |
|---|---|
| 首页、NyxBot 页的方案卡；方案列表、筛选、相关阅读、相关文章中的方案链接 | 继续进入方案详情，不禁用浏览 |
| 每个方案详情的“使用此方案 / Use this solution” | 默认 `<button disabled>`，文字保留、灰色样式、无 href、无 handler、无 hover 光效和位移 |
| 禁用按钮的辅助信息 | 旁边显示“暂未开放 / Not available yet”，不承诺开放日期；无需 hover 才能看到 |
| 鼠标、触屏、Enter、Space | 禁用 CTA 都不能跳转或启动流程；不能只加灰色 CSS |
| 全站登录、开始使用、NyxBot 通用入口、工作原理、定制套餐联系 | 保持原可用行为，不全局禁用主按钮 |
| 旧 `#/start?t=…` 或 `/start?t=…` 分享入口 | 未开放方案不得绕过开通门槛，也不得显示模拟“已上线”；先识别并回到对应详情的未开放状态。无方案参数的原通用认证入口按既有行为处理 |
| 某一个方案验收通过 | 只启用其明确的方案配置和真实承接路径，其余继续禁用；不能一键全开 |

Solution discovery remains available. Only activation controls are disabled, with native disabled semantics and visible availability text. Generic authentication and product entry points remain usable. Legacy scheme-specific start URLs must not bypass availability checks or claim successful setup. Enablement is per solution, never a global assumption.

| 初始方案清单 / Initial inventory | 默认开通状态 / Default activation |
|---|---|
| `restaurant-manager` | 禁用 / Disabled |
| `help-desk` | 禁用 / Disabled |
| `appointment-booker` | 禁用 / Disabled |
| `callback-queue` | 禁用 / Disabled |
| `stock-analyst` | 禁用 / Disabled |
| `news-aggregator` | 禁用 / Disabled |
| `desk-researcher` | 禁用 / Disabled |
| `x-lead-catcher` | 禁用 / Disabled |
| `back-office-clerk` | 禁用 / Disabled |

上表表示**本次未收到任何一项已通过完整验收的证据，故默认关闭**，不表示 NyxID 缺少这些底层能力。现有认证、NyxBot、Telegram 创建与连接应复用。开放需要保留所选方案上下文，走完认证、配置、授权和一次真实任务；TG 已连接只是其中一步。

These defaults reflect the absence of supplied end-to-end acceptance evidence, not the absence of underlying NyxID capabilities. Reuse existing authentication, NyxBot and Telegram integration; activation requires the selected solution to survive authentication and complete configuration, authorization and a real task.

### 登录落点：维持当前行为 / Preserve authentication behavior

已登录访问首页 `/` 仍去 `/dashboard`；普通登录无有效指定目标时仍使用现有默认落点。有合法 `return_to`、邀请码或 OAuth 参数时，继续沿用现有校验与传递，不能把所有路径强制覆盖为 dashboard。仅首页既有会话重定向保持，不额外给 Blog、Contact 或方案详情加登录拦截。

Authenticated visits to `/` still go to `/dashboard`; ordinary sign-in keeps the existing default destination. Valid return targets and authentication context retain their existing validation and handling. Do not add authentication gates to public Blog, Contact or solution details.

后续“直接去 NyxBot”是单独的产品路由变更：指登录后的工作区，**不是官网介绍页 `/nyxbot`**。当前产品侧使用 `/assistant`；未来落点、方案上下文协议和安全返回规则需另行确认和验证，本期不预先切换。

A later direct-to-NyxBot change means the authenticated workspace, **not the marketing `/nyxbot` page**. The current product uses `/assistant`; the eventual route and context contract need separate confirmation and testing.


### 2026-10-21 发布开关与恢复操作 / Publication switches and restoration

集中配置为 `frontend/src/features/website-a/release-config.ts`，沿用轻量 Vite build-time 环境变量。下列四个开关（含 Blog 菜单）相互独立，必须显式为字符串 `true` 才请求开启；默认均为 false。请求开启但缺少合法 adapter、地址或非敏感验收记录引用时，配置解析报错，完整构建失败，不生成可发布 release。没有 runtime 自动探测或新的配置后台。

Configuration is centralized in `release-config.ts` using lightweight Vite build-time environment variables. Each switch defaults off and requires the literal string `true`. Requested enablement with incomplete/invalid adapter settings or an acceptance reference fails the complete build. There is no runtime autodetection or new configuration service.

| 功能 / Feature | 默认开关 / Default switch | 开启依赖 / Required configuration |
|---|---|---|
| Blog 菜单 / Blog menu | `VITE_WEBSITE_BLOG_MENU_ENABLED=false` | 仅恢复顶部/移动菜单；直接访问、Footer 与正文审核保持独立 |
| Newsletter | `VITE_WEBSITE_NEWSLETTER_ENABLED=false` | `VITE_WEBSITE_NEWSLETTER_ENDPOINT`、`VITE_WEBSITE_NEWSLETTER_PRIVACY_URL`、`VITE_WEBSITE_NEWSLETTER_UNSUBSCRIBE_URL`、`VITE_WEBSITE_NEWSLETTER_ACCEPTANCE`；已验收的确认邮件/退订/滥用与数据处理链路 |
| Contact NyxBot | `VITE_WEBSITE_CONTACT_CHAT_ENABLED=false` | `VITE_WEBSITE_CONTACT_CHAT_ENDPOINT`、`VITE_WEBSITE_CONTACT_CHAT_ACCEPTANCE`；已验收的公共问答服务、限额/限流及失败处理 |
| Footer Status | `VITE_WEBSITE_STATUS_ENABLED=false` | `VITE_WEBSITE_STATUS_URL`、`VITE_WEBSITE_STATUS_ACCEPTANCE`；真实 HTTPS 公共状态页，不能指向官网首页或 `/health` |
| 单个方案 / Each solution | `VITE_WEBSITE_SOLUTION_ACTIVATIONS={}` | 只添加已验收 slug 的 `{"target":"/assistant?真实已验收参数","acceptance":"验收记录引用"}`；目前仅允许现有 `/assistant`、`/dashboard` 工作区目标，不新建开通协议 |

Newsletter/Chat 的 endpoint 是明确配置的同源 `/api/...` 路径，adapter 使用无 cookie/凭证的 POST JSON，不复用个人 NyxBot 会话或 Agent Key。Newsletter 请求 `{email,locale}`，只接受 `{status:"pending_confirmation"}`，UI 仅说明请求接收、等待邮件确认；不能把 HTTP 200 当订阅完成。Chat 请求 `{message,locale}`，只接受非空 `{reply}` 并按纯文本显示，不接原型脚本回复。两者服务端均尚未接通；fixtures 只验证接线。超时 10 秒，响应上限 16 KiB，失败保留输入与邮件/Discord 联系渠道；SSR 表单在 hydration 前禁用，避免默认 GET 泄露内容。

The configured same-origin `/api/...` adapters send credential-free JSON POST requests. Newsletter sends `{email,locale}` and accepts only `{status:"pending_confirmation"}`; the UI does not claim a completed subscription. Chat sends `{message,locale}` and accepts a nonempty plain-text `{reply}`. Neither live backend is connected; fixtures prove wiring only. Requests time out after 10 seconds and cap responses at 16 KiB. Failures retain input and contact alternatives, and SSR controls remain inert until hydration.

恢复步骤：①负责人确认真实后端、隐私/退订或状态地址与目标环境的完整验收记录；②只配置需要开放的单项，将其开关改为 `true`，方案只添加该 slug；③运行完整 `npm run build` 和针对性验收，确认客户端、HTML、Markdown、llms/sitemap、旧入口及认证上下文一致；④发布同一 SHA 的完整产物。`public-release.json` 记录有效 configuration、visibility 与 configRevision；HTML/客户端在 hydration 前校对相同有效配置，拒绝混装，不能仅替换客户端脚本。关闭状态不渲染组件、不留空框、不发相关请求；默认 bundle 同时排除三个可选组件分支。普通可切换 tab 仍沿用批准内容的完整 Markdown 规则，不受这些发布开关影响。

To restore: obtain service-owner acceptance, configure only the selected feature/slug, run the complete build plus targeted checks, then publish the complete artifact from one SHA. The release manifest records effective configuration, visibility and configRevision; hydration refuses mixed configurations; do not replace JavaScript alone. Off means no rendered component, placeholder or feature request, with optional component branches removed from default bundles. Ordinary approved tab content retains its full Markdown representation.

一键关闭回滚：将故障单项开关设为 `false`，或从方案 JSON 删除该 slug，重建并发布完整产物。其它功能保持原值；不需要从历史 Commit 找回页面，不恢复假回复、假“已发送”、未审核文章或未验收开通。`ACCEPTANCE` 只是非敏感验收引用，无法替代人工真实验收；所有 `VITE_` 配置均为公开构建配置，严禁放入密钥或内部敏感信息。

To turn a feature off, set its switch to `false` or remove that solution entry, rebuild and publish the complete artifact. Other switches remain unchanged. Never restore fake replies, simulated sends, unreviewed articles or unverified activation. An acceptance reference does not replace human testing; all `VITE_` configuration is public and must contain no secrets or sensitive internal data.

新增验收：8 种三开关组合的独立性；默认 off 的 SSR/客户端/Markdown/bundle 排除和零请求；单项 on 的合法 adapter 响应、失败/取消和无 JS 防护；配置缺失/非法拒绝完整构建；九个方案逐项配置及 legacy gate 一致；既有 mailto、导航 query、dashboard 和可信 return_to 回归。真实服务未验收前维持关闭。

Additional checks cover all eight switch combinations, default-off rendering/bundle/request boundaries, on-state adapters and failures, inert no-JS forms, invalid configuration rejection, per-solution legacy gates, and existing contact/navigation/auth regressions. Keep production switches off until real services pass acceptance.

## 4. 用户场景与方案取舍 / Users and implementation options

| 用户 / User | 本期需要完成的事 / Job to be done | 对应取舍 / Scope implication |
|---|---|---|
| 中小商户 / Merchant | 看懂能做什么，找到适合自己的方案及真实联系入口 | 场景与详情保留；未开放开通按钮明确不可用 |
| 第三方施工队 / Installer | 判断某个方案能否交付，并复用现有配置能力 | 逐方案真实验收；不把示意图当交付证据 |
| AI 爱好者、开发者 / Enthusiast, developer | 继续探索产品、集成与资料 | 保留通用注册、文档、集成和 NyxBot 入口 |
| 内容维护者 / Editor | 本期发布准确内容，后续再获得自主发布工具 | 仓库 PR 静态维护，CMS 后移 |

六项产品决定已固定，以下只比较静态 Blog 的维护实现，不重新讨论是否建设 CMS。

The six product decisions are fixed. The alternatives below concern static Blog maintenance, not whether to build a CMS now.

| 静态维护选项 | 收益 | 代价与边界 | 建议 |
|---|---|---|---|
| A：沿用现有页面模板，明确正式静态文章集和单一可编辑源，通过 PR 发布 | 最少模板变更，保持 v4.1 视觉，便于先交付 | 编辑仍需研发/Git 协作；生成文件与源必须同步，不能直接改生成结果后被覆盖 | **本期推荐** |
| B：本期先整理成独立 Markdown/MDX 内容目录，再发布静态页 | 文章维护更集中，未来迁移边界更清晰 | 增加内容迁移、渲染与旧链接回归；不自动带来运营后台 | 文章规模或维护频率证明需要时再做 |

Option A retains templates with an explicit reviewed static content source and PR publication. Option B introduces a Markdown/MDX content structure first, adding migration and regression work. Recommend A for this release; neither option claims a CMS exists.

## 5. 里程碑、依赖与估工 / Milestones, dependencies and effort

| 阶段 | 交付与完成条件 | 依赖 | 粗估 |
|---|---|---|---|
| M0 本次评审 | 独立 v4.2 文档、v4.1 对照图、范围与验收表 | 产品确认裁剪口径 | 本次文档交付 |
| M1 官网裁剪 | 静态 Blog 发布边界、订阅/聊天/Status 默认关闭且保留源码、单项发布配置、9 项禁用、旧入口防绕过；中英文与移动端复验 | 内容负责人确认公开文章；有效 Discord 配置；现有认证行为 | 约 2–4 人日研发与回归，按现有模板复用估算，不含内容重写、真实方案接通 |
| M2 按方案开放 | 每次 1 个方案满足下节全部条件；仅开放该项 | 方案负责人、产品确认、目标环境账号/所需渠道、现有服务与权限 | 每个方案单独拆解，当前证据不足以统一估时 |
| M3 后续服务 | CMS 成熟后接入；公共 NyxBot 有对外版本后再接；登录后落点另案变更 | 各服务负责人及产品排期 | 单独估工；本次不承诺时间 |

Estimate M1 at approximately 2–4 engineering person-days using existing templates, excluding content rewrites and operational use-case integrations. Validate the estimate during implementation planning. Later integrations require individual owners, dependencies and estimates; no dates are committed here.

Newsletter、Contact NyxBot 和 Status 本期默认隐藏，不设自动恢复期限；组件及接口边界保留，后端上线不会自动开启。分别完成订阅/退订服务、公共聊天服务、公共状态页的独立验收，再填写配置、重建和发布完整产物；不用寻找旧 Commit 恢复源码。联系表单继续使用已确认的邮件客户端方式，不把邮件后台列为 M1 前置阻塞。

Newsletter, public Contact chat and Status remain default-off with their source retained. A backend launch never enables them automatically. Accept each real service, configure its switch and adapter, then rebuild and release the complete artifact; no old-commit recovery is needed. The existing mail-app contact flow remains, so a new mail backend is not an M1 dependency.

## 6. 验收标准 / Acceptance criteria

| 检查项 | 通过标准 |
|---|---|
| Blog 静态阅读 | 中英文列表、分类、详情、相关阅读与合法旧链接可用；正文和来源已审核；未知/未发布文章明确不可读；公开包无草稿 |
| Newsletter | 中英文和移动端均无邮箱订阅控件、订阅占位或周更/退订承诺；Discord 目标正确 |
| Contact | 默认输出无聊天窗口、快捷问题及聊天输入，源码保留；单项开启仅调用经过验收的 adapter；表单五字段、校验、bundle 预选、邮件地址和 Discord 正确；无假“已发送” |
| Footer | 默认各页均无 Status，占位空隙消除；组件及配置保留；条款、隐私与其它有效链接保留 |
| 登录回归 | 未登录可浏览公开页；已登录首页 → dashboard；有效 return_to 与原有认证参数行为不变；无真实登录验收证据时不得写“认证端到端通过” |
| 方案默认状态 | 9 项 × 中英文开通 CTA 禁用；鼠标/触屏/键盘不可触发；详情浏览和全站通用 CTA 不受影响；带方案旧链接不能绕过 |
| 单方案启用 | 已登录、已注册未登录、新注册三种路径均保留所选方案，按线上可用认证方式完成；复用或建立所需渠道与服务授权，确认任务配置，并取得一次真实任务结果；失败/取消不误报成功 |
| 启用证据 | 每项记录目标环境、版本、方案 slug、验证人、日期、操作路径和结果；产品确认后开放；不能仅以静态截图、HTTP 200 或已有 TG 能力作为证据 |
| 视觉回归 | 对照 v4.1 保持已批准样式；禁用按钮无诱导光效；桌面和移动端排版无空框/溢出；首页及工具墙不新增连续动画 |

Acceptance covers reviewed static articles, default-off retained optional UI, intact contact/authentication behavior and per-solution activation gates. Enabling a solution requires recorded evidence for signed-in, signed-out and new-user paths plus configuration and a real task outcome. Static screenshots and HTTP status checks alone are insufficient.

## 7. 指标、风险与回滚 / Metrics, risks and rollback

北极星指标：**已有证据、可重复交付的已开放商户方案数**。后续有试点时补充“开始配置 → 首个真实任务成功”的完成率；本期不虚构基线、商户数或转化提升。静态官网发布不是该指标已经增长。

North star: the number of enabled merchant solutions with evidence of repeatable delivery. Once pilots exist, measure setup-to-first-task completion. Do not invent adoption or conversion baselines or equate website publication with delivery progress.

护栏：未验收方案误开放数为 0；已开放方案验收记录覆盖率 100%；变更入口死链、虚假发送成功、公共包草稿泄漏和认证行为回归为 0。本期通过测试/验收记录统计，不新增分析平台或收集访客数据。

Guardrails: zero unverified activation, broken changed links, false send confirmations, bundled drafts or authentication regressions; complete acceptance evidence for enabled solutions. Use test and acceptance records without adding an analytics service.

| 风险或假设 | 验证与处理 |
|---|---|
| 原型文章含示例、未经确认的承诺 | 内容负责人审核正式文章清单；保留示例性质，不在本轮擅自编写案例 |
| 只隐藏按钮，旧链接仍能触发方案流程 | 按 slug 做默认关闭判断，同时验证直接地址和鼠标/键盘入口 |
| 默认 dashboard 与未来方案 return_to 混淆 | 分开测试普通登录、已登录首页、指定返回目标；不全局覆盖有效返回路径 |
| 无默认邮件客户端或无法判断实际投递 | 保留明确 mailto 说明、可复制邮箱及未发送的填写内容；不宣称已送达 |
| 新裁剪误伤 NyxBot 产品能力或全局 CTA | 仅对 Contact 聊天组件做默认关闭，禁用仅指方案开通；逐个核对其它入口 |
| 文章生成流程覆盖人工改动 | 确定单一静态内容源；源与生成结果同一 PR；必要时做确定性生成检查 |

Roll out the approved changes through the existing feature-branch and fork-PR workflow; organization/upstream PR creation remains with the user. Deploy only after the implementation and acceptance steps are complete. Disable a failing solution independently. Revert static content through a PR when required, but never restore prototype fake-success flows, unreviewed drafts or unverified activation merely to reproduce v4.1.

上线遵循现有功能分支与 fork PR 流程，组织/上游 PR 继续由用户发起；实际实施与验收完成后再发布。单个方案故障优先单独关闭。静态文章通过 PR 撤回；回滚不能恢复原型假成功、未审核草稿或未验证的开通入口。

## 8. 证据与本次产物边界 / Evidence and artifact limits

- v4.1 源：`website-review/nyxid-ste-preivew-new.html`，Blog 1264–1287、Contact 1372–1401、Status 933、Use case 1056；本次未修改。
- 当前实现参照：`frontend/scripts/website-a/generate.mjs`、`frontend/src/features/website-a/contact.tsx`、`frontend/src/router.tsx`、`frontend/src/main.tsx`。
- Blog 历史设计：`docs/BLOG_IMPLEMENTATION.md`；mock 状态不能替代当前正式发布设计。
- [视觉对照与静态页面切片](website-scope-v4.2/review.html)：Before 从 v4.1 模板提取；After 为本次裁剪评审稿。截图表示默认关闭后的视觉目标，不代表本轮最新运行截图，也不证明源码删除、登录、发信、聊天或方案真实可用。
- [复现与校验记录](website-scope-v4.2/evidence.json)：记录基线文件摘要、产物、静态检查与截图条件。

The review assets compare extracted v4.1 templates with the proposed scope revision. They show the default-off visual target, not fresh runtime screenshots or proof of source removal, authentication, mail/chat delivery or solution execution. Existing v4.1 materials remain unchanged.
