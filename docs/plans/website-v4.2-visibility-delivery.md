# v4.2 暂时隐藏交接 / Temporary visibility handoff

2026-10-21。基线 `c17f8c8fe0aa23eb2892377af3be1f6c0bf51a52`，交付分支 `aelf-hzz780/NyxID:codex/website-a-delivery`。此前 SEO 复验仅绑定该旧基线，本轮是独立增量。目标是默认不展示、保留可恢复源码、验收后逐项配置发布；不接真实后端、不部署、不改变认证和 dashboard 默认落点。

Dated 2026-10-21, based on `c17f8c8fe0aa23eb2892377af3be1f6c0bf51a52`. Earlier SEO acceptance applies only to that baseline. This increment retains recoverable implementation behind default-off publication settings, without connecting live services, deploying, or changing authentication/default dashboard behavior.

## 改动与开关 / Changes and switches

| 项目 / Item | 默认配置 / Default | 保留实现及恢复条件 / Retained implementation and enablement |
|---|---|---|
| Blog 顶部/移动菜单 | `VITE_WEBSITE_BLOG_MENU_ENABLED=false` | `optional-features.tsx` 的 BlogMenuLink；设为 true 并重建即可恢复菜单。Footer Blog、直接访问、已审核内容及既有索引策略不改 |
| Newsletter | `VITE_WEBSITE_NEWSLETTER_ENABLED=false` | `newsletter-signup.tsx` + HTTP adapter；确认/退订/隐私及滥用控制链路验收后填 endpoint、privacy URL、unsubscribe URL、acceptance 引用，再显式开启 |
| Contact 公共 NyxBot 卡 | `VITE_WEBSITE_CONTACT_CHAT_ENABLED=false` | `contact-chat.tsx` + HTTP adapter；公共无凭证服务、配额/限流及失败处理验收后填 endpoint、acceptance，再显式开启 |
| Footer Status | `VITE_WEBSITE_STATUS_ENABLED=false` | `footer-status.tsx`；真实 HTTPS 公共状态地址与验收引用就绪后配置，不能用官网首页、API 或 health 冒充状态页 |
| 九个方案 | `VITE_WEBSITE_SOLUTION_ACTIVATIONS={}` | `solution-activation.tsx`/共享 legacy resolver；只添加已通过完整真实任务验收的 slug、target、acceptance，其余继续 disabled，详情页均保留 |

集中位置：[release-config.ts](../../frontend/src/features/website-a/release-config.ts)；环境变量示例：[.env.website.example](../../frontend/.env.website.example)。合并所需设置到既有 build 环境，不覆盖其它配置。`VITE_` 值都是公开构建配置，不能包含凭证或敏感内部材料。开关仅认字符串 `true`；请求开启却缺配置或验收引用时，配置解析令完整构建失败。后端上线不会自动开放。

Settings are centralized in the linked configuration and example. Merge settings into the existing build environment. Every `VITE_` value is public and must contain no credentials or sensitive material. Only literal `true` requests enablement; incomplete requested settings fail the complete build. A backend launch never flips a switch.

展示与服务之间采用小型 Adapter 边界，不建设 feature-flag 后台。生成器输出稳定组件插槽，默认构建通过静态分支排除休眠表单组件，不用 CSS 或 hidden 隐藏公开 HTML。组件配置关闭时返回 null，无占位、监听器或请求。普通批准 tab 内容继续完整进入 Markdown，不套用这些发布开关。

Small adapters separate UI from services; no flag-management service is added. The generator emits stable component slots, and static build branches exclude dormant forms. Off means no rendered element, placeholder, listener or request. Ordinary approved tab content retains its complete Markdown representation.

## 可执行 adapter 契约 / Executable adapter contract

同源 `/api/...` endpoint，无 cookie、Authorization 或个人会话凭证。POST 带随机 `X-Request-ID`，错误只记录该 ID 和固定诊断，不记录邮箱、问题、响应正文或配置地址。10 秒超时、16 KiB 响应上限，组件卸载/失败均取消 transport，React 按文本显示回答。

Adapters use configured same-origin `/api/...` endpoints without cookies, authorization or personal sessions. Requests carry a random request ID; diagnostics contain neither email/questions nor response bodies. A 10-second timeout, 16-KiB response limit, transport cancellation and text-only rendering bound failures.

- Newsletter：请求 `{email,locale}`；仅接受 `{status:"pending_confirmation"}`。提示只表示请求接收、等待邮件确认，不宣称订阅完成或邮件送达。
- Chat：请求 `{message,locale}`；仅接受非空 `{reply}`，限 4000 字。没有原型 regex 回复或模拟 transcript。
- 两种表单 SSR 时完整 fieldset/submit 禁用，hydration 完成才启用；失败保留输入。既有 Contact 五字段 mailto、`topic=bundle` 与 Discord 继续可用，不恢复假“已发送”。

Newsletter accepts only a pending-confirmation acknowledgement; chat accepts a bounded nonempty plain-text reply. Neither uses simulated success or scripted demo responses. Both forms are inert until hydration, preserve input on failure, and retain the existing mailto and Discord alternatives.

Blog 的六组双语候选正文源码、旧列表/详情模板与 CMS 设计仍保留，未批准内容继续被 publication manifest 与最终 bundle 扫描隔离。未来 CMS 的接入边界是审核后内容源 → 正式文章 adapter → 同一公开发布清单；当前 build 仍要求先实现/验收正式文章 adapter，不会因开关或 CMS 上线自动批准候选。没有新增正文、文章路由或索引政策。

Candidate article source, legacy list/detail templates and CMS design remain available. The future integration boundary is reviewed content → formal article adapter → the same publication allowlist. CMS availability or visibility settings never approve pending content. This increment adds no article routes or indexing-policy changes.

## 恢复发布与关闭回滚 / Enablement and rollback

1. 服务负责人完成目标环境真实验收，并保存可核对记录。Newsletter 包含确认、发送、退订与数据处理；公共聊天包含限额/滥用控制，不能使用个人助手权限；方案需跨登录保留上下文并完成真实任务。
2. 只配置所选项。Newsletter/Chat 填对应 `_ENDPOINT`、`_ACCEPTANCE`（Newsletter 另需隐私/退订 HTTPS 地址）；Status 填 `_URL`、`_ACCEPTANCE`；Blog 菜单只改独立开关。方案 JSON 只加已验收项，例如结构 `{ "help-desk": { "target": "/assistant?已验收的真实参数", "acceptance": "公开非敏感验收引用" } }`。这只是配置结构示例，不证明当前 assistant 支持任何示例参数。
3. 在同一环境运行完整 `npm run build`、相关回归及目标环境验收。客户端、预渲染、Markdown、llms、sitemap、路由作为同一个 release 发布；不要单独替换 JS。manifest 保存有效配置、configRevision、contentRevision；hydration 前逐值核对有效配置，拒绝相同布尔开关但接口/target 不同的混装。
4. 关闭故障项只需设对应开关为 false；方案从 JSON 去掉该 slug。重建并发布完整产物，其它功能保持。无需从旧 Commit 恢复组件，不恢复假回复、假成功或未审核内容。

Obtain live-service acceptance, configure only the selected item, validate and publish a complete rebuilt artifact. Effective configuration and revisions bind all representations, and hydration rejects mixed settings. To turn an item off, change that switch/remove the solution entry and rebuild. Other items remain unchanged; no source recovery from history is needed.

## 验证与边界 / Validation and limits

本机只运行串行单测、TypeScript noEmit 与源码生成/静态文档检查，不运行完整构建/浏览器，不开 4180，不改现有服务。测试包含三服务开关 8 组合、Blog 菜单独立开关、中英 SSR/Markdown、单项 on、缺配置报错、pending acknowledgement、真实格式校验、失败/超时/取消、九方案隔离及 legacy 入口、配置混装拒绝、单项 bundle 残留门禁，以及既有 Contact/导航/query/auth 回归。HTTP responses 均为隔离 fixture，不代表后端上线。

Local verification is limited to serial tests, TypeScript and source/static checks, with no local full build/browser or service changes. Tests cover switch independence, bilingual rendering, adapters and failures, per-solution gates, mixed configuration, dormant-bundle exclusion and existing contact/navigation/auth behavior. Mock responses establish wiring only, not live availability.

隔离 CI 将运行完整构建、90 页公开产物门禁、HTTP/日志隐私及单 worker 浏览器。产物门禁按有效配置检查每项 marker/目标 URL、菜单与开通 CTA，并继续扫描全部未审核文章。新增浏览器覆盖桌面/移动与中英 Blog 菜单、直接页面和 Footer 保留，按 manifest 检查默认可见性及浏览期间无相关写请求。开启的服务还需真实目标环境验收；本轮生产默认全部关闭。

Isolated CI performs the full build, 90-page artifact checks, HTTP/privacy and single-worker browser tests. Gates check effective markers/URLs, menu and activation controls, dormant feature code and unreviewed article exclusion. Live enabled services still need their own accepted target-environment evidence; current production defaults remain off.

最终工程 SHA：**`fd00726ec086ed5cf3d8ad19a20e88a00555a2b0`**。[最终 CI](https://github.com/aelf-hzz780/NyxID/actions/runs/39578181573) 全通过：30 文件 / 232 单测、完整 TypeScript/生产构建、90 页产物、219 HTTP、日志隐私、10 项单 worker browser（0 skipped / 0 flaky）。[完整 artifact](https://github.com/aelf-hzz780/NyxID/actions/runs/39578181573/artifacts/12335496628) 已下载核对 digest：`sha256:d0f0bc5adb532428f9022061a9a8b896540a3527d1fcd69d057d1917c57fad32`。manifest、源码 SHA 和配置快照一致：四开关 false、solutions 空集合。contentRevision 为 `49e686770de9ed0f5a924923048c88611956e8d8ae7af93bb73744fa9ba236b1`，configRevision 为 `a27cd071131edbef9665b638f622bf1a541f90673ee733d077c77f71e2faef27`。机器证据见 [verification.json](website-visibility-evidence/verification.json)，精简原始日志见 [ci-proof.txt](website-visibility-evidence/ci-proof.txt)。最终文档 Commit 只补证据，交付 HEAD 以包含本记录的最后 Commit 为准。

Final engineering source **`fd00726ec086ed5cf3d8ad19a20e88a00555a2b0`** passed the linked CI: 232 tests in 30 files, complete build, 90-page gate, 219 HTTP checks, privacy and ten browser tests without skips/flakes. The complete artifact was downloaded and its digest, source SHA and all-off configuration verified. The final documentation commit records evidence only; delivery HEAD is the last commit containing this handoff.

| 新增 Commit / Commit | 内容 / Change |
|---|---|
| `325068a8d` | 可恢复组件、独立发布配置、Blog 菜单隐藏、逐项方案 / Retained components and publication controls |
| `46b3a5a78` | 配置 snapshot/revision、混装拒绝、产物与浏览器门禁 / Configuration-bound artifact and hydration checks |
| `cbdcaa53e` | v4.2 原方案与原评审页/生成器同步 / Revised scope and reproducible original review |
| `72500d639` | 恢复 Blog 导航时保留 active/query；适配默认隐藏断言 / Blog restoration context and menu assertions |
| `fd00726ec` | 拒绝首页空 query/hash 的 Status 变体 / Reject homepage variants as Status destinations |
| 最终文档 Commit / Final documentation commit | 核验后的交接与 artifact 证据 / Verified handoff and artifact evidence |

最新 lab 单样本 `/security`：reduced-motion=true，73 resources，559843 transfer bytes，LCP 56ms，CLS≈1.951。只记录观察，CLS 仍值得单独排查；没有运行性能矩阵，也不宣称性能门槛、field CWV 或增长结果通过。

The single lab sample records reduced-motion=true, 73 resources, 559843 transfer bytes, LCP 56ms and CLS≈1.951. High CLS remains a separate concern. No performance matrix, field-CWV or growth outcome is claimed.

独立验收由主对话转交；不把旧基线的复验结论沿用为本轮通过。真实后端、已审核新文章、实际方案接通和 field CWV 不在本轮完成范围。

Final source/artifact evidence is recorded above. The main chat submits independent acceptance; prior results do not establish this increment's acceptance. Live services, newly approved articles, operational use cases and field CWV remain outside this delivery.

## 文档与材料归属 / Document ownership

同步并只选择跟踪：本交接、指定 v4.2 方案、原 review.html、其 build-review.py 与 README。补改生成源是为了避免重新生成覆盖新口径。既有图片与静态切片保留原样，本轮未重拍；文档中的图片仍是历史 default-off 视觉目标，不能冒充最新运行截图。

This commit selectively tracks this handoff, the requested v4.2 proposal and original review, plus its generator/README to prevent stale regeneration. Existing images/excerpts are preserved, with no new browser capture; they remain historical default-off visual targets.

主对话维护的 [visibility-review.html](website-scope-v4.2/visibility-review.html)、`source/build-visibility-review.py`、`visibility-evidence.json` 不由本轮编辑或暂存。其余原有未跟踪方案/评审资产保持原状。旧 [SEO 交接](website-seo-ai-delivery.md) 保持历史可追溯；没有 merge、部署或代提组织 PR。

The separate visibility review, generator and evidence belong to the main chat and are neither edited nor staged here. Other pre-existing untracked materials remain untouched, and the prior SEO handoff stays historical. No merge, deployment or organization PR is performed.

历史图片与切片仍按要求保持未跟踪，因此单独 checkout 本次新增 review 文本并不包含其全部本机图片依赖；主对话的视觉材料后续提交由主对话负责。正式前端构建和四项开关的恢复不依赖这些原型文件，已生成的页面与可恢复组件均有受版本管理的源码。

Historical images/excerpts remain untracked as requested, so this commit's review text alone does not include every local image dependency. The main chat owns subsequent visual-material submission. Production builds and feature restoration use tracked component/generated source and do not require those prototype assets.
