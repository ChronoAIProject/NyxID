# 官网 A + v4.1 交付 / Website A + v4.1 delivery

当前状态：实现及主题修复已完成本地验证，等待独立验收；**原场景品牌水印尚未解决，整体不能判定完成**。组织 PR 由负责人手动创建。本文覆盖先前 A-only 文档里的“不接 B”旧范围。

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

`cd frontend && npm ci && npm run dev -- --host 127.0.0.1 --port 4180 --strictPort`；预览 `http://127.0.0.1:4180/?lang=zh`。生产构建不依赖未跟踪的评审源文件。

The local preview is retained on port4180. Production builds require only committed source and locked dependencies.

`VITE_WEBSITE_BACKGROUND_RUNTIME_URL` / `VITE_WEBSITE_BACKGROUND_PROJECT_ID` 允许配置已授权资源；`VITE_WEBSITE_VISUALS_ENABLED=false` 可重建为静态增强回退。需要撤回本次代码时，通过正常 revert Commit 回到上述 A 基线，不修改应用主题存储，不部署或强推。

The background URL/project can be configured for authorized resources. A build-time visual flag provides static resilience. Use a normal revert to restore accepted A; no preference reset, force push, merge or deployment is required.

## 增量配色修正 / Brand-gradient correction

负责人追加：首页中英文强调词与 How-it-works 的“安全地”统一为 `#A672FB → #5E00F5`，移除浅白/灰紫色 stop。保留9秒 GradientText，静态/reduced-motion/离屏回退统一复用 `var(--vivid)`；未改参照页、文案、字号或布局。

The Home emphasis now uses the reference page's two brand-purple endpoints while retaining the nine-second GradientText animation. Static fallbacks reuse the original vivid token. The reference page, content and layout are unchanged.

增量验证：13项相关测试、改动 ESLint、68个输出生成复现通过；真实浏览器核对中英动画、中文参照页和离屏静态 computed gradient 一致，返回后动画恢复。见 `website-v4.1-evidence/color-audit.json` 与 `color-*.jpg/txt/json`。整体125项/完整构建记录属于前一实现 commit；本纯配色修改没有重跑全量。水印阻塞不变，独立验收按新 commit 增量复验。

Incremental checks passed:13 related tests, scoped lint and68-file generation equality. Browser evidence records both animated languages, the reference and static fallback, plus animation recovery. Earlier full-build/125-test evidence belongs to the previous implementation commit; no full rerun was needed for this palette-only change. Branding remains unresolved.
