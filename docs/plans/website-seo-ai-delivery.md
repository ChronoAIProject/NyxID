# 官网 SEO / AI 阅读交接 / Website SEO and AI reading handoff

本次以 `aelf-hzz780/NyxID` 的 `codex/website-a-delivery` 为交付分支。基线 `942a64b2d9af35aea01448c1ba6c3a62697472a2`。本文件记录工程交付与实际验证，不代表已经部署、被搜索引擎收录或通过独立验收。组织 PR 由人工提交。

Delivery targets `aelf-hzz780/NyxID:codex/website-a-delivery`, based on `942a64b2d9af35aea01448c1ba6c3a62697472a2`. This records implementation and actual checks, not deployment, indexing or independent acceptance. The organization PR remains a human action.

## Commit 分工 / Commit breakdown

| Commit | 内容 / Change |
|---|---|
| `33cb551ba` | 发布审批与 v4.2 业务边界 / Publication and availability gates |
| `f18b0cfc4` | 独立公共树、hydration、双语持久导航 / Public tree and navigation |
| `b70de2d0c` | HTML/Markdown、Nginx、CI 门禁 / Static representations and release gates |
| `697691449` | 修复实际 bundle 泄露与中文示例 / Prototype chunks and localized demo |
| `53f958465` | 测试镜像、内部链接、已登录路径验收 / Artifact packaging and authenticated paths |
| `7aa338ad8` | Router 原生 SSR 边界一致性 / Native router SSR boundary alignment |
| `5fc812e9d` | 访问日志 query 脱敏与验证 / Access-log query privacy |

## 范围与边界 / Scope and boundaries

已审核 JSX 页面是官网正文源；Docs 沿用既有 manifest + Markdown，法律页沿用既有 Markdown。公共 renderer 只加载公共图，不导入完整应用入口。浏览器先接管同样的树，再恢复 query、原有 session、consent 和 Docs 控件。第三方 API/OAuth/MCP/health 仍由原代理边界处理。

Approved JSX is the marketing body source; Docs retain their manifest/Markdown, and legal pages retain their Markdown. The public renderer excludes the full application entry. The browser hydrates the same tree before restoring visitor context, session, consent and Docs controls. Existing API/OAuth/MCP/health proxy boundaries remain authoritative.

19 个已批准官网页面 × 中英两种语言，另有 50 个英文 Docs 页面（含索引）及 2 个法律页，共 90 个 canonical HTML。Blog 索引明确等待审核并 noindex；6 篇新候选及 5 篇旧 mock 文章未获正式审批，全部不发布。它们仍保留于源码，开发工具仅在 DEV 编译分支可用。新增正式文章需正文、locale、作者/日期及审批 adapter；当前构建会拒绝仅改状态发布候选文章。

There are 19 approved marketing pages in two locales, 50 English Docs pages including the index, and two legal pages: 90 canonical HTML outputs. Blog indexes disclose pending editorial review and are noindex. Six new candidates and five legacy mock articles are excluded. Their source remains, with prototype tooling confined to DEV branches. Formal articles require approved body/locale/author/date data and an adapter; status-only publication is rejected.

9 个方案的 Use this solution 为原生 disabled，显示未开放；旧 start 参数回对应详情，未知方案回列表。Contact 仅五字段 mailto 表单及 Discord，保留 `topic=bundle`；Newsletter 与脚本聊天已撤下。首页已登录仍回 dashboard，登录本身及 trusted return_to 校验继续使用原应用代码。

All nine solution activation controls are native disabled buttons with visible availability notes. Legacy start links resolve to their disabled detail or the list. Contact retains the five-field mailto form, Discord and `topic=bundle`; newsletter and scripted chat are removed. Authenticated homes still redirect to dashboard, while login and trusted return handling retain the existing implementation.

## 任务映射 / Task mapping

| Task | 工程交付 / Engineering delivery | 验收边界 / Acceptance boundary |
|---|---|---|
| T01 | `frontend/content/publication.json` 清点；`published.json` 为安全公共投影 / inventory and safe projection | 候选文章 pending / Articles pending |
| T02 | JSX → 同一 renderer → HTML/Markdown；Docs 白名单 / One body source and allowlist | 正式文章 adapter 随真实输入补齐 / Article adapter awaits editorial input |
| T03 | 独立 SSR build、预载正文、显式 hydration marker / Public-only SSR and hydration | 单浏览器 CI + 独立验收 / Browser CI and independent acceptance |
| T04 | 英文原路径、`/zh`、逐页 head、WebPage schema / URLs, head and schema | 无虚构作者、价格、客户背书 / No invented claims |
| T05 | manifest exact HTML → 实际 app pattern → 404；缓存、安全头、proxy / Exact routing and HTTP contracts | 打包 Nginx 实测 / Packaged Nginx checks |
| T06 | `/read/{en,zh}/...md`、索引、官网 llms.txt / Markdown and machine index | API llms playbook 不变 / API playbooks unchanged |
| T07 | root 外壳、body pending/error、TanStack Link、焦点/历史 / Persistent shell and navigation | 延迟 chunk、locale、back/forward / Delayed chunk and history |
| T08 | 全集/metadata/MD revision/草稿/链接门禁；原子打包 / Artifact gates and atomic package | 失败无 release artifact / Failed validation blocks release |
| T09 | 未实施外部编辑和搜索运营 / External editorial/search operations deferred | 需真实账号、内容与上线 / Requires owners and release |
| T10 | consent 事件、基线表、20题双语协议、lab 采样 / Events and measurement preparation | 真实 GSC/Bing/AI/field 数据 not-run / External results not-run |

## URL 与 HTTP / URL and HTTP

英文保持原路径，中文使用 `/zh`。仅对已知官网路径处理 `lang=en|zh` 与末尾 slash，使用 301 单跳并保留其它 query。重复 lang（含大小写混用）拒绝为 400；无效 locale 参数不创建新页面。Docs/法律没有虚构译本；auth/protocol query 不进入营销归一规则。业务 query 重复时 SiteLink 保留原生 href，避免改变已有参数顺序或含义。

English keeps existing paths; Chinese uses `/zh`. Only known marketing paths normalize valid locale queries and trailing slashes, with one-hop 301s preserving other parameters. Repeated locale parameters, including case variants, return 400. Invalid values create no language variant. Docs/legal retain English; authentication/protocol parameters are untouched. Duplicate business parameters retain native navigation to preserve semantics.

HTML/Markdown 使用 no-cache + ETag 再验证；hash assets immutable；app shell no-store + noindex。Markdown 镜像 noindex，不进 sitemap。未知公开、预览和未审批文章返回真实 404。已知 app 动态路径来自 router 声明，不能用任意 `/admin/*` 或 `/settings/*` 兜底。预览构建设置 `PUBLIC_SITE_ORIGIN` 后 head noindex；非正式 Host 的 HTML 也带 noindex 响应头。预览访问控制仍由部署 owner 配置。

HTML/Markdown revalidate; hashed assets are immutable; private app shells are no-store/noindex. Markdown mirrors are excluded from the sitemap. Unknown, preview and unpublished article routes return real 404s. Valid app patterns come from router declarations rather than broad namespace fallbacks. Preview builds use `PUBLIC_SITE_ORIGIN` for noindex heads, and nonproduction HTML hosts receive a noindex header. Deployment owners remain responsible for preview access control.

## 实际验证 / Actual validation

- 本机仅运行受控单 worker 测试、TypeScript 和限定文件 ESLint；未运行完整 build、浏览器或新服务，未操作 4180。 / Local verification used bounded unit tests, TypeScript and focused ESLint, without a full build, browser, new server or changes to 4180.
- 公共/官网测试已有 123 项通过，后续新增双语 demo 两项已单独通过；最终 CI 数量与结果以对应 SHA 日志为准。 / 123 public/website tests passed locally, followed by two added localized-demo cases; the final CI log is authoritative for its source SHA.
- TypeScript 通过；限定文件 ESLint 无 error，存在 3 条 Fast Refresh 结构警告。 / TypeScript passed; focused ESLint had no errors and three structural Fast Refresh warnings.
- 第一轮 CI `b70de2d0c` 生成全部 90 页，但最终扫描抓到旧 lazy registry 发出的未审核文章 chunk，正确阻止 release 打包；修复为 DEV-only 导入。第二轮 `697691449` 通过最终产物门禁，在测试镜像打包时发现 dist 被 ignore；已加 Dockerfile 专用白名单。 / The first CI rendered all 90 pages but correctly blocked packaging on legacy prototype chunks; their imports are now DEV-only.

最终工程源码 **`5fc812e9df18da1d1c2b9538535c6c4f57840813`**，content revision **`d8f61ba7e05cb45364432358dbd46255749a78d2c96368c5191994cb6021f825`**。2026-10-20 UTC 的[最终 CI](https://github.com/aelf-hzz780/NyxID/actions/runs/39409952720)全部通过：15 文件 / 125 项单测、完整 TypeScript/生产构建、90 页产物门禁、219 项串行 HTTP 检查、请求/referrer 日志隐私探针、5 项单 worker 浏览器检查。最后的文档 Commit 仅补充交接和测量模板，不改变此工程源码。

The final engineering source is **`5fc812e9df18da1d1c2b9538535c6c4f57840813`**, with content revision **`d8f61ba7e05cb45364432358dbd46255749a78d2c96368c5191994cb6021f825`**. [Final CI](https://github.com/aelf-hzz780/NyxID/actions/runs/39409952720) passed all gates: 125 tests across 15 files, the complete TypeScript/production build, 90-page artifact validation, 219 serial HTTP checks, query-log privacy, and five single-worker browser tests. The final documentation commit does not change this engineering source.

[完整产物](https://github.com/aelf-hzz780/NyxID/actions/runs/39409952720/artifacts/12259545591)的 GitHub artifact digest 为 `sha256:90b28b34272d467a24756c02f91f9741f2371f44bd83f1c89cf3a20300df7449`。机器记录与精简原始日志见 [verification.json](website-seo-evidence/verification.json) 和 [ci-proof.txt](website-seo-evidence/ci-proof.txt)。第三轮真实浏览器发现 Router 顶层 Suspense 不一致，修复原生 SSR marker 后第四轮及最终轮全通过。全部失败均未产生 release artifact。

The [complete artifact](https://github.com/aelf-hzz780/NyxID/actions/runs/39409952720/artifacts/12259545591) has GitHub artifact digest `sha256:90b28b34272d467a24756c02f91f9741f2371f44bd83f1c89cf3a20300df7449`. Machine evidence and concise log excerpts are stored alongside this document. The third run exposed a router Suspense mismatch; native SSR-marker alignment passed both subsequent runs. Failed runs never produced a release artifact.

独立验收尚未运行；登录回归使用 CI backend fixture，未实测真实 OAuth provider。lab 样本不代表 field CWV；GSC/Bing/AI 引用/人工咨询均未采集。测量准备见 [README](website-seo-measurement/README.md)、[baseline.csv](website-seo-measurement/baseline.csv) 与 [20题双语问题](website-seo-measurement/ai-questions.json)。原有本机未跟踪方案输入保持原状，未混入本次 Commit。

Independent acceptance and live OAuth-provider testing have not run; authentication checks use a CI backend fixture. The lab sample does not establish field CWV, and external search, AI-citation and inquiry results remain uncollected. Measurement templates are linked above. Existing untracked planning inputs were preserved and excluded from these commits.

## 独立验收入口 / Independent acceptance inputs

在 CI 的完整产物中核对 `public-release.json`、`public-validation.json`、`public-http-validation.json`、`public-browser-results.json`、`public-lab.json` 与两份 Nginx includes。对照实际 SHA，复查 `/`、`/zh`、`/security`、`/zh/security`、`/solutions/help-desk`、`/zh/contact?topic=bundle`、`/docs/ai/guides/agent-isolation`、`/read/en/home.md`、`/llms.txt` 以及未知路径、合法 app deep links。不要在 Codex 中打开 4180；隔离产物使用 4188。

Inspect the complete artifact's release, validation, HTTP, browser and lab JSON plus both Nginx includes. Verify the source SHA and the representative routes above, unknown paths and valid application deep links. Do not open 4180 in Codex; isolated artifact checks use 4188.

## 上线与回滚 / Launch and rollback

未执行合并、部署、站长提交或组织 PR。上线应使用同一 SHA 的完整镜像/产物，将 HTML、JS/CSS、Markdown、llms/sitemap 和 Nginx 配置一起发布，验证后再切流；构建失败不替换旧版本。触发回滚的情况包括空正文、错语言、登录参数丢失、draft 泄露或主要路径持续失败。

No merge, deployment, webmaster submission or organization PR was performed. Release all representations and Nginx configuration from one validated SHA as a complete image/artifact, then switch traffic. Roll back for empty bodies, wrong locales, lost authentication context, draft disclosure or persistent route failures.

回滚必须恢复完整上一版本；若 `/zh` 或永久跳转已经对外发布，保留可达的兼容目标和精确路由映射，不能简单退回不认识新 URL 的旧镜像。保留旧 hash assets 至旧页面缓存过期；不要只退 JS，或依靠 robots 清除已泄露资料。未审批文章、未开放方案、未采集外部指标及既有背景供应商 watermark 均为明确剩余边界。

Rollback must restore the complete previous release while preserving reachable `/zh` destinations and compatibility mappings for cached permanent redirects. Retain old hashed assets for cached documents. Do not roll back JavaScript alone or use robots as a disclosure remedy. Pending editorial content, disabled solutions, uncollected external metrics and the existing background-provider watermark remain explicit limitations.
