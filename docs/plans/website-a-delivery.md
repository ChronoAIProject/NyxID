# 官网方案 A 交付 / Website A delivery

## 目标与范围 / Goal and scope

将产品负责人原始 HTML 的主要页面、内容、三类访客和展示交互集成为现有 React 官网；复用现有真实登录与注册。此次不接入视觉 B，不新增匿名试用、免费额度、CMS、订阅或联系后台，不改鉴权、权限与计费。

Integrate the baseline content, audiences and display interactions into the existing React site. Reuse production authentication. No B enhancements, anonymous trial, free credits, CMS, subscription/contact backend, or auth/billing changes.

## 基线与依赖 / Baseline and dependencies

- Upstream base: `8280098785b477a9980cc8aed98f43789d6fd3dc` (2026-10-15 fetched).
- Branch: `codex/website-a-delivery`.
- Product source: `docs/plans/website-review/nyxid-site-preview.html`, SHA-256 `5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33`.
- Existing untracked review/proposal/archive files are preserved and excluded from the implementation commit.
- No spec-kitty marker found. Existing React, TanStack Router, i18next, CSS and test infrastructure are reused.

## 完成标准 / Acceptance criteria

1. 首页、NyxBot、Solutions 与九个详情、How it works、Security、Your agent、Integrations、Pricing、Blog 与六篇文章、Contact 可直接访问，未知路径有 404；内部设计评审页不公开。
2. 中英文保留 A 的主要内容和结构；原型模拟业务与真实能力有明确差异记录。
3. 首页 9 秒轮换、点击停止、hover 暂停、focus 停止和键盘切换可用；NyxBot 示例、方案/博客/集成筛选和代码切换可用。
4. 已有登录、注册、query、可信 return_to、OAuth、MFA、首页登录态跳转保持；不实现原型模拟登录和完成页。
5. 构建、类型检查、相关 lint/测试与真实浏览器桌面/移动、中英文检查有实际证据。
6. 交付 commit、fork 分支与明确验证记录交到独立验收对话；组织仓库 PR 由负责人手动创建，不自行合并或部署。

Direct public routes, bilingual content, original display interactions, production authentication parity, concrete build/browser evidence and a commit-scoped independent acceptance handoff are required.

## 页面映射 / Route mapping

| Product prototype | Production |
| --- | --- |
| `#/` | `/` (existing auth guard retained) |
| `#/nyxbot`, `#/solutions`, nine solution details | Matching pathname routes |
| `#/how-it-works`, `#/security`, `#/agents`, `#/integrations`, `#/pricing`, `#/contact` | Matching pathname routes |
| `#/blog`, six article slugs | Existing `/blog` route surface with baseline content |
| `#/login`, `#/signup` | Existing `/login`, `/register` with query preserved |
| `#/start`, `#/done` | Existing registration entry; no simulated provisioning/success |
| design/brand/reference guides | Internal review only, excluded |
| Terms / Privacy | Existing `/terms`, `/privacy` |

## 登录与跳转基线 / Authentication baseline

线上 `https://nyx.chrono-ai.fun/` 于 2026-10-15 实际打开：登录 `/login`，注册和首页主 CTA `/register`，抢先体验 `#beta`，Discord `https://discord.gg/QMvcs8UQBW`，Docs `/docs`。该浏览器为未登录状态，不能据此推断已登录分支。

The production home was opened on October 15, 2026 in an unauthenticated browser. Links above were read from the rendered DOM. Authenticated branches require code/tests, not inference from this session.

代码证据：`features/landing/components/landing-navbar.tsx` 与 `hero.tsx` 将完整 location.search 带入登录/注册；`landing-page.tsx` 对 Reddit 来源且无 code 时注入既有邀请归因。`router.tsx` 首页已认证且加载结束跳 `/dashboard`；auth layout 对可信 return_to 执行原返回，否则 `/dashboard`。`pages/login.tsx` 和 `register.tsx` 保留 AuthFlow、MFA、邀请与社交错误处理。本轮不改这些认证模块。

Code preserves complete query propagation, the existing Reddit attribution fallback, authenticated root redirect, trusted auth returns, AuthFlow and MFA. Authentication modules remain authoritative.

## 风险、决策和回滚 / Risks, decisions and rollback

- 原型有匿名试用、演示 bot/QR、静态联系成功、订阅与 Start/Done。发布时不能宣称未发生的成功；入口映射已有注册/社区，缺失后台的动作明确不可用。差异逐项记入交付。
- 内容与 CSS 从固定哈希的 A 源迁移；生产页面使用 React 与 pathname routing，不嵌入评审 HTML、不执行原型脚本。样式限于官网，避免影响登录、控制台与法律页。
- 外部字体和服务图标有网络依赖，名称和系统字体作为回退。
- Rollback: revert the implementation commit/PR. No data migration or backend/config changes.

Prototype-only actions cannot claim production success. Public styles stay scoped; external assets retain readable fallbacks. Revert the implementation commit to roll back.

## 验证记录 / Verification

首次独立验收发现 A-01～A-03，已按下方复验记录修复；独立复验结论待验收对话给出。最初移动端工具限制已解除，本轮实际测得 390×844。旧原型 QA 不计作正式实现通过证据。

The first independent acceptance found A-01–A-03. Fixes and targeted evidence are recorded below; independent re-acceptance is pending. The initial mobile tooling limitation has been resolved with an actual 390×844 viewport. Prior prototype QA is not counted.

| Check | Result and evidence |
| --- | --- |
| Targeted unit/regression tests | 10 files / 85 tests passed, covering auth query, Reddit attribution, public route boundaries, existing Login/Register/MFA props, trusted returns, home redirect, showcase timing/focus/reduced motion, film playback and docs helpers. See `website-a-evidence/tests.txt`. |
| Production build | `npm run build`: TypeScript, Vite, legal prerender, credential-accept build and mock-footprint assertion. See `website-a-evidence/build.txt`. |
| Changed TypeScript ESLint | Passed; see `website-a-evidence/lint.txt`. This is targeted lint, not a claim that all unrelated repository code is lint-clean. |
| Direct routes, desktop | All 25 routes × English/Chinese = 50 checks, each has its expected heading, native main and zero iframes; no horizontal overflow at actual 1280×720. See `website-a-evidence/desktop-route-audit.json`. |
| Authentication parity | Actual production and local preview expose Google, GitHub, Apple and NyxID app. Production query propagation matches local root → Solutions → Login. Evidence: `auth-parity.json`, `online-login-query-before.jpg`, `local-login-query-after.jpg`. |
| Interactions | NyxBot chip selection, MCP code switching, blog filtering, unavailable newsletter, original five-workflow selection/playback and keyboard audience switching. See `interactions-audit.json` and screenshots. |
| Navigation edges | Unknown solution and unknown public route show 404; existing blog article still renders; legacy Start becomes real registration; `#beta` scrolls to the community section. See `route-edges.json`. |
| Tool wall v4.1 | Static 24 cards, original order, six columns, 10px gap; first card 175.664×58px with 12px/14px padding and 12px radius. See `tool-wall-audit.json`. |
| Mobile browser | Initial run was blocked (`viewport-override-not-applied.jpg`); this is historical. Independent acceptance subsequently checked all 25 routes × two languages at actual 390×844 and desktop. This fix run also measured actual 390×844, verified both expanded-menu login links and keyboard navigation/query preservation. See `website-a-evidence/fix-1/browser-audit.json`. This is browser evidence, not a physical-device test. |

## 首轮验收修复 / First acceptance fixes

首轮独立验收未通过，发现 A-01（窄屏无直接登录）、A-02（自定义方案卡遗漏 CTA/样式）、A-03（NyxBot 侧栏徽标溢出）。本轮仅修复这三项；验证和截图见 `website-a-evidence/fix-1/README.md`。验收结论由指定独立对话给出，不由实现方自行宣布。

The first independent acceptance failed on mobile login reachability, the custom solution CTA/styles, and sidebar badge overflow. This follow-up fixes those three findings only. Targeted checks and screenshots are in `website-a-evidence/fix-1/README.md`; the assigned acceptance chat owns the re-acceptance conclusion.

## 已确认的生产适配 / Explicit production adaptations

| Prototype behavior | Delivered behavior |
| --- | --- |
| Mock social login / invitation regex / Start→Done | Original `/login` and `/register`, full query preserved. No mock account or provisioning success. Legacy hash conversion uses an exact public-route allowlist; protocol-relative/external inputs are rejected. |
| “No sign-up / try now” plus unset demo bot QR links | Neutral connect/setup copy and existing registration. Unavailable demo links and decorative QR are excluded. No anonymous trial or free credit entitlement. |
| Contact form local “sent” and scripted chat | Contact page uses the existing real Discord destination. No fake submission, no contact backend or email collection. |
| Newsletter button with no handler | Clearly disabled “Subscriptions coming soon”; no data sent or saved. |
| Illustrative Luna Bistro story | Retained with an explicit example/non-customer disclosure. |
| Internal design/brand/reference pages and placeholder status | Excluded from public navigation. Existing terms/privacy and Docs/CLI remain reachable. Unverified hosting/certification placeholders ask the team instead of asserting compliance. |
| Solutions film as embedded HTML document | Original five scenes and bilingual data rendered as native React, with selection and working play/pause. Three-column explanation is retained; responsive stacking is implemented. The exact original frame-by-frame typing/wire choreography is not reproduced. |
| Moving audience-tab indicator | Equivalent selected tab treatment with the original colors/shape. Rotation, hover/focus, keyboard, staged checks/messages and progress remain. No B visual effects. |
| Legacy blog URLs | All five existing article slugs remain routed to their existing renderer; the six A articles use the new website. |
| Existing early-access anchor | `#beta` remains a real Discord early-access section and scrolls after lazy page mount. |

最新工具墙要求来自来源对话的已确认 v4.1：24 项静态网格，桌面六列/窄屏两列，原 Logo/名称/顺序/尺寸/hover；本轮没有 DriftWall、循环副本、裁切或暂停键，也没有替代动画。24 项不是完整目录的 145 项。

The confirmed v4.1 tool-wall decision is incorporated: 24 static entries with original geometry and hover, six desktop/two narrow columns, no continuous motion or replacement effect. The full 145-entry catalog remains separate.

## 实现与审查 / Implementation and review

- Native JSX is generated once from the exact approved source and committed, split by locale/page for lazy loading. Builds do not need review files. Generator/provenance: `frontend/scripts/website-a/README.md`.
- A separate i18next instance avoids overwriting auth/app resources. Public selectors and every keyframe name are scoped; static brand SVGs are local. Third-party fonts/icons have readable fallbacks.
- Read-only Reviewer and Verifier checks found and resolved query loss across navigation, trailing slash admission, keyframe collision and a potential hash open redirect. Unit tests cover the redirect rejection. A focused failing playback test reproduced a focus/pause issue before its fix; the final test passes.
- `docs-page.tsx` uses explicit `loaded !== null` checks in two existing expressions so the complete TypeScript build remains valid with the expanded route registry. Rendering behavior is unchanged.

只读审查属于实现阶段质量控制，不代表指定独立验收对话已经通过。

The read-only reviews are implementation quality checks, not acceptance by the separately assigned acceptance chat.

## 运行与复验 / Run and reproduce

Use a project-supported Node runtime (this run used the installed Node 20.20.2):

```sh
cd frontend
npm ci
npm run dev -- --host 127.0.0.1 --port 4180 --strictPort
npm run test -- src/features/website-a src/lib/public-paths.test.ts src/router.test.ts src/pages/login.test.tsx src/pages/register.test.tsx src/lib/return-url.test.ts src/features/docs
npx eslint src/features/website-a src/lib/public-paths.ts src/lib/public-paths.test.ts src/pages/landing.tsx src/pages/blog-detail.tsx src/router.tsx src/router.test.ts src/features/docs/docs-page.tsx
npm run build
```

Public pages are at `http://127.0.0.1:4180/?lang=zh` or `?lang=en`. A missing local backend shows the existing login-method error; this is not a replaced/mock login. To inspect production public sign-in configuration only, the separate preview was launched with:

```sh
BACKEND_URL=https://nyx-api.chrono-ai.fun FRONTEND_URL=https://nyx.chrono-ai.fun npm run dev -- --host 127.0.0.1 --port 4181 --strictPort
```

No real login, OAuth consent, registration, message, purchase or account mutation was performed. Authenticated session and external OAuth/MFA completion were checked by existing code/tests, not a real account run. Proxy TLS/public-config failures and HMR restarts occurred during development; the final clean page showed no JS error in the recorded check. This is not a universal zero-console-error claim.

## PR 与验收交接 / PR and acceptance handoff

- `origin`: `git@aelf.github.com:aelf-hzz780/NyxID.git`; SSH and connector identity both verified as `aelf-hzz780`.
- `upstream`: `git@aelf.github.com:ChronoAIProject/NyxID.git`.
- `origin/main` is 12 upstream commits behind the verified base. A non-protected `codex/website-a-base` review branch mirrors that exact base for a delivery-scoped fork comparison. `main` is never pushed directly.
- 按负责人最新确认，本轮通过实现 commit 并推送 `aelf-hzz780/NyxID` 的 `codex/website-a-delivery` 分支交付；组织仓库 PR 由负责人手动创建，不要求先创建 fork PR。
- Per the owner's latest instruction, implementation commits pushed to the fork delivery branch are the delivery artifact. The owner will create the organization PR manually; a fork PR is optional.
- Independent acceptance chat: `01a13edf-2b31-75c1-b1ca-824d707f02ff`. Handoff identifies the exact delivery commit, fork branch and current verification evidence. Independent re-acceptance of the fixes remains pending.

### PR 准备与责任 / PR preparation and ownership

交付代码已提交并推送到 fork。此前连接器创建 PR 返回 403；负责人随后明确改为 fork commit 交付、手动创建组织 PR，因此无需开放连接器 PR 写权限，也不再将自动创建 PR 视为交付阻塞。本轮未创建、合并 PR 或部署；英文 PR 描述与对比链接保存在 `website-a-pr.md`，可供负责人使用。

The implementation and both branches are pushed. After an earlier connector 403, the owner explicitly chose fork commit delivery and manual organization PR creation. Connector PR write access is no longer required or a delivery blocker. No PR was created or merged, and nothing was deployed. The ready English description and compare links are in `website-a-pr.md`; independent acceptance reviews the exact commit.

## 剩余风险 / Remaining limits

1. Actual 390px browser navigation has now been checked, including the repaired login menu. Physical-device touch behavior, device performance and OS reduced-motion transitions remain unverified. Unit tests and simulated media changes are not OS/device evidence.
2. The 145-entry catalog and pricing/business statements retain the approved product source. Individual third-party connection availability and commercial policy were not exhaustively exercised; no anonymous/free-use entitlement is added.
3. Fonts and provider favicons depend on external networks. Full offline, WebKit and Firefox runs are not claimed.
4. No real account sign-in, external provider consent, MFA completion or authenticated session browser regression was executed. Existing auth code/guards are retained and covered by targeted tests.
5. Deployment requires the existing SPA fallback for the new pathname routes. Current Nginx already uses it. Rollback is a code revert; there are no data migrations.
