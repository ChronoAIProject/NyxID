# 官网分支 main 同步与署名交接 / Website main sync and authorship handoff

2026-10-22（Asia/Shanghai）。本轮合入精确上游 `cefb3e674a2a4364cf5e42c897d9c22659bcbce0`，采用双亲 merge，保留全部独立提交，不 rebase/squash。基线为 `acd7e381d595ade5598f0a2b3742fb406b857a59`。已完成的 v4.2、SEO 和文字对齐验收只绑定各自原版本，不替代本轮融合后的验证。

This delivery merges the exact upstream SHA above with both parents retained. Earlier v4.2, SEO and text-alignment acceptance remains tied to its original version and does not establish acceptance of this integration.

## 冲突与自动合并决策 / Conflict and automatic-merge decisions

| 文件 / File | 融合决策 / Resolution |
|---|---|
| `frontend/nginx.conf.template` | 保留上游 CSP/Permissions/HSTS、无 query/Referer 日志、独立 proxy 策略、credential-accept 前缀和较大 channel reply body 上限；接入官网生成的明确公开/应用路由、locale/canonical 重定向、Markdown/llms 与真实 404。每个生成 location include 完整安全头，防止 nginx 1.27 继承被截断；renderer 同时渲染 routing marker，支持自定义 root/output。 / Combine the response-specific upstream policies with explicit website routes and full per-location headers. |
| `frontend/src/main.tsx` | 保留官网轻量 dispatcher 和资源失败恢复；前置上游 zod-CSP、personal/onboarding handoff，使凭证先清理再加载应用。Root/QueryClient 继续在 `app-entry.tsx`，账号切换的 telemetry dependency 移植到实际 Root。 / Preserve split entrypoints and initialize upstream CSP/handoffs before application loading. |
| `frontend/src/router.tsx` | 同时保留上游 PersonalLinkPage、`/nyxbot/link` 和新 usage 权限行为，以及官网真实 `/start` 的共享方案认证入口；顶层 code、完整可信 return_to 与默认 dashboard 逻辑不回退。 / Retain both new upstream routes and the verified solution/auth handoff. |

自动合并核对：package/lock 均为 frontend 0.73.0，根 dependencies/devDependencies 完全一致；上游 Tiptap 依赖保留。Dockerfile 保留新 Node/runtime policy 链并补齐 legal-cache；测试 Dockerfile 复用同一 renderer/entrypoint/default.conf，dockerignore 与 artifact tar 同步包含所需文件。`public-paths` 同时保留官网路径与 `/nyxbot/link`；telemetry schema、router tests、Vite zod alias 与 claim referrer 改动保留。

Automatic merges retain the upstream dependency versions, routes, telemetry changes and CSP aliases. Production/test images use the same policy renderer; ignore rules and packaged files include its runtime dependencies.

Legal 两页只有一组生成 exact location，保留上游 GET/HEAD、24h must-revalidate 和内容 ETag。最后增强 metadata 的 HTML 写入后重新计算 SHA256，避免旧 ETag 错误返回 304。HTTP gate 对实际响应正文核对 ETag，并覆盖 304/405、完整安全头以及 credential/API proxy 自身策略。

Legal pages have one generated exact location each, retaining upstream method/cache/ETag behavior. Hashing the final metadata-enhanced HTML prevents stale 304s; HTTP checks validate exact response hashes and response-specific policies.

两轮只读审查指出的 dockerignore 漏项和 production-policy-server Docker 参数误替换已修复。未展开第三轮审查。整树 diff-check 另发现上游原有 mobile EOF 空行，未为本次官网融合修改该无关文件；融合相关 frontend/workflow diff-check 通过。

The two-round read-only review findings were fixed. An existing upstream mobile EOF whitespace issue was retained rather than changing unrelated code; scoped frontend/workflow diff checks passed.

## 署名重写与恢复 / Authorship rewrite and recovery

人类明确授权本功能分支的姓名改为 `Richard.Huang`，并仅为该目的允许精确 lease 的历史重写。审计范围是 `HEAD --not cefb3e674…` 的 43 个分支独有提交；其 author/committer 均为黄宗哲。只替换这两处姓名，邮箱、时间戳、消息与每个 tree 全部逐项验证不变；上游可达提交不重写。repo-local `user.name=Richard.Huang`、`user.email=huangzongzhe@MacBook-Pro.local`，未修改 global。

The explicit human authorization covers only the 43 feature-only commits. Both names changed, while emails, dates, messages and trees remain identical. Upstream-reachable commits were excluded; only repository-local identity defaults were set.

原 merge `811e3c206054d8ab27bfe9f37fce850e1ccc9fe7` → 新 merge `b277ef7d6cca0187839f6174be4546294286bcf4`；两者 tree 均为 `a4ec0ce6aab77d65ea321a802318f51399e00962`。新 merge 双亲：改名后的官网分支父提交 `3b0a4762937a08f7a199b1e3386aa54864176c02` 与未重写的 upstream `cefb3e674…`。

The old/new merge mapping and common tree are above. Both merge parents remain, with the upstream parent unchanged. Full per-commit mapping is in [author-rewrite-map.json](website-main-sync-evidence/author-rewrite-map.json).

本地恢复引用 `refs/backup/website-before-richard-20261022` 指向原 merge，`refs/backup/website-alignment-before-richard-20261022` 指向旧文字对齐版。远端仅使用：`--force-with-lease=refs/heads/codex/website-a-delivery:811e3c206054d8ab27bfe9f37fce850e1ccc9fe7`；目标是 fork `aelf-hzz780/NyxID:codex/website-a-delivery`，没有普通 force、main push、PR merge 或部署。

Local backup refs preserve the original merge and alignment version. The remote rewrite used the exact old-SHA lease only against the authorized fork feature branch. No upstream history, main branch or deployment was changed.

## 工作树与范围 / Worktree and scope

合并在 `/Users/huangzongzhe/.codex/worktrees/website-main-sync/nyxid` 的 `codex/website-main-sync` 完成；原 `/Users/huangzongzhe/workspace/vibeCoding/nyxid` 仍停在旧 `acd7e381d`。未触碰原有未跟踪视觉资料、node_modules 或运行服务；没有本地重构建、全量测试、浏览器或 4180 操作。旧样式验收已 PASS，但不会冒充合并版视觉验收。

The managed isolated worktree contains the integrated source. The original checkout, dependencies, untracked materials and running service remain untouched. Earlier alignment PASS does not establish merged-version acceptance.

安全同步：当前服务仍使用旧 checkout；先由主对话协调服务切换窗口，再从新远端创建独立本地分支，例如 `git switch -c codex/website-a-delivery-synced --track origin/codex/website-a-delivery`，随后在服务未使用依赖期间执行 `npm ci`，最后由主对话按既定方式运行服务。不要对运行中的旧 checkout 执行 reset/rebase 或边运行边替换依赖。本任务不执行这些服务操作。

For safe synchronization, the main chat must coordinate a service handover first. Create a new local tracking branch from the rewritten remote, install dependencies while the service is not consuming them, then restart through the established owner flow. Do not reset/rebase the active checkout or replace live dependencies. This task does not perform those service actions.

## Wizard 源闭包配套 / Embedded wizard source closure

W-F01：本分支新增 GSAP 等 lockfile 内容，使 `wizard_bundle_freshness.rs` 的 EXTRAS 输入变化。按原算法复算：expected `8be48051a5b3f4ac469466fc5cf1e6922722ec35c8354ef60acaaf251acf7356`，旧 recorded `5188b13f28916bbc3f17827eeb1e5d4f95307a599dc3253226bae3f751e76922`。这是真实源闭包不匹配；不是已运行 Cargo 的声明，也不能手改 hash 掩盖。

The complete lockfile is a canonical freshness input. The independently recomputed mismatch above requires an actual wizard rebuild; it is not a claim that Cargo already ran.

已追加 `e2d37551b` 和 `a8ad970f6`，在官网 default/on 之后顺序生成 wizard 并上传实际产物；producer 自身生成的 hash 必须与 HEAD 记录匹配，之后运行未改动的原始 `cargo test -p nyxid-cli --test wizard_bundle_freshness`。实际生成产物另作 Richard.Huang 独立 Commit，最终验证绑定该 Commit 后的 SHA。

Two CI commits add serial generation/upload and comparison using the existing producer's source-closure hash, followed by the unchanged authoritative Rust test. Generated artifacts are committed separately; final verification must identify the resulting SHA.

实际 CI 生成产物已由 `58a45da9735cc6ce442a17ef501f5fea3e484789` 独立提交；仅 HTML 和生成的 hash 变化，178 文件 manifest 不变。HTML 的 JavaScript 块不变，CSS 块变化。生成源为 `c99e2310366d5d13629b14dd3939786744e0b8cb`，完整来源、逐文件与脚本/样式块摘要见 [wizard-regeneration.json](website-main-sync-evidence/wizard-regeneration.json)。

The actual CI-produced bundle is committed separately at `58a45da9735cc6ce442a17ef501f5fea3e484789`. Only the HTML and producer-generated hash change; the 178-file manifest and JavaScript block remain identical. The provenance record identifies the generating source and per-file/block digests.


## 严格 CSP 下的 SSR 兼容修复 / Strict-CSP SSR compatibility

`a8ad970f6` 的 run `39714990135` 已通过单测/构建/91页/227 HTTP，但 browser 为 9 pass、1 fail。实际失败是 TanStack SSR 的 ScrollRestoration 输出自删除内联脚本，被新 CSP 阻止后残留 DOM 导致 hydration mismatch；未通过过滤错误或放宽 CSP 掩盖。`c99e23103` 单独修复：SSG memory router 不输出该脚本，浏览器 router 继续启用历史滚动恢复；新增窗口无关 renderer 断言和最终产物内联脚本门禁。

The first integrated browser run exposed a real strict-CSP incompatibility despite passing unit/build/HTTP gates. The independent fix removes only the SSG inline restoration requirement, retaining browser restoration and strict CSP. Final-source CI evidence below supersedes this failure without erasing it.


## 最终版本与验收证据 / Final source and verification

工程源码 `58a45da9735cc6ce442a17ef501f5fea3e484789` 的 [CI run 39717322160](https://github.com/aelf-hzz780/NyxID/actions/runs/39717322160) 已全部成功（2026-10-22 03:42:40 UTC）。本说明与证据随后以 docs-only commit 交付；该文档提交不是重建来源，不把旧 SHA 的结果标为新构建。完整可机器读取的结果、配置/内容指纹、下载摘要核验和署名复核见 [final-verification.json](website-main-sync-evidence/final-verification.json)，原始日志摘录见 [final-ci-excerpts.txt](website-main-sync-evidence/final-ci-excerpts.txt)。

All four jobs succeeded for the exact engineering source above. The following documentation-only commit does not change that tested source; artifacts retain their actual generating SHA. Machine-readable evidence and raw log excerpts are linked above.

| 阶段 / Stage | 最终结果 / Final result |
|---|---|
| Default public artifact, job `119622452645` | 6 nginx policy tests; 38 files / 338 unit tests; complete TypeScript/Vite build; 91-page publication gate; 227 HTTP checks and log privacy; 10 browser tests, 0 skipped/flaky/unexpected. |
| Single solution on fixture, job `119623588907` | 91-page gate; only help-desk enabled; 2 browser tests, 0 skipped/flaky/unexpected; real `/start` to Login/Register invite + return-target wiring covered. |
| Actual wizard regeneration, job `119624347687` | `npm run build:wizard` succeeded; 178-file manifest; producer-generated source hash equals the committed hash. All downloaded generated files, including HTML, are byte-identical to the committed bundle. |
| Authoritative committed wizard test, job `119624817459` | Actual `cargo test -p nyxid-cli --test wizard_bundle_freshness`: `wizard_bundle_is_fresh ... ok`; 1 passed, 0 failed. Original Rust test and Node installer remain unchanged from upstream. |

默认四个可选入口关闭、solutions 为空；on-fixture 仅启用 help-desk，目标 `/assistant?setup=help-desk&draft=hello+world`，并明确 `productionDelivery=false`、`realTaskAccepted=false`。两个产物的配置与内容 SHA256 已从实际下载的 manifest 独立复算，ZIP 摘要也与 GitHub 记录一致。

The default artifact keeps all four optional features and every solution activation off. The separate on fixture enables only help-desk for wiring verification, explicitly without production delivery or operational acceptance. Configuration/content hashes and downloaded ZIP digests were independently verified.

| 产物 / Artifact | ZIP SHA256 |
|---|---|
| [Default `12399965752`](https://github.com/aelf-hzz780/NyxID/actions/runs/39717322160/artifacts/12399965752) | `cb5d2e7ce73884ec1bede746eb7431f6359881049e5c46304f15345a7a375190` |
| [On fixture `12399304163`](https://github.com/aelf-hzz780/NyxID/actions/runs/39717322160/artifacts/12399304163) | `10056819f10d57c382a4673f392cc5fd7ea666e7a02e14a61a25f03f1b4fe636` |
| [Wizard `12399199884`](https://github.com/aelf-hzz780/NyxID/actions/runs/39717322160/artifacts/12399199884) | `833e4fb1ad2f926e1835285e73914cb111cf2380cdc3dd3cceaff3edf602af56` |

后续独立提交：`e2d37551bd7bd1fb56478bb1006f63a2cb13b37d` 增加串行 wizard CI；`a8ad970f6046fd480a2502eb11106ea62cafc40c` 纳入 committed wizard 触发路径；`c99e2310366d5d13629b14dd3939786744e0b8cb` 修复严格 CSP 下的静态 hydration；`58a45da9735cc6ce442a17ef501f5fea3e484789` 提交真实重建产物。43 个历史提交的反向对象审计全部通过；最终工程源码共有 47 个分支独有提交，author/committer 均为 Richard.Huang，邮箱不变。

The independent follow-up commits add serial CI, cover artifact-trigger paths, repair strict-CSP hydration and commit the actual regenerated wizard. Reverse-object reconstruction passed for all 43 renamed commits; all 47 feature-only commits at the tested engineering source have the requested author/committer name and retained email.

验收边界：未运行全仓 `cargo test` / `npm run test`；未执行合并版真实账号、OAuth/MFA/provider、线上 solution 业务任务或主对话的定向视觉复验。CI 的 LCP/CLS 只是单浏览器 lab 记录，不能视为 field CWV 达标。原 checkout、4180、依赖和未跟踪资料未变。本轮没有创建 PR、部署或重启服务；PR 模板草案见 [website-pr-description.md](website-pr-description.md)。

Verification is limited to the stated suites. Full repository tests, live account/provider flows, operational solutions and owner acceptance of the merged visuals remain outstanding. Single-browser lab metrics do not establish field CWV performance. The original checkout and live service remain untouched; no PR, deployment or service restart was performed.
