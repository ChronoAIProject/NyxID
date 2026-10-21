# v4.2 首轮 FAIL 修复交接 / First-FAIL repair handoff

2026-10-21（Asia/Shanghai）。首轮正式独立验收为 **FAIL**；本次为其第一次修复派发，下一次为第 2 轮正式独立复验，尚未执行。工程测试与 CI 成功不代表独立验收通过。

The first formal independent acceptance **failed**. This handoff covers its first repair dispatch; formal round-two independent acceptance remains pending. Engineering tests and CI do not establish independent acceptance.

仓库/分支：`aelf-hzz780/NyxID:codex/website-a-delivery`。首轮交验 HEAD `ed91d302ff560388315b6cc1149491bb9d5beb95`，对应工程 `fd00726ec086ed5cf3d8ad19a20e88a00555a2b0`。本次工程 SHA **`44bde4d23ded1cba2d7747e00bc146d5feaf0c96`**；最后补充证据的文档 Commit 不改变工程源码。

Repository/branch and source identifiers are above. The final documentation-only commit records evidence without changing the engineering source.

| 发现 / Finding | 修复与验证 / Repair and verification | Commit |
|---|---|---|
| V-F01 P2 Status 首页别名 | 拒绝 `/`、`/zh`、`/zh/`；按 URI 解码、重复斜杠与尾斜杠规则规范化后再次比较，覆盖 `/zh//`、`/%7A%68`、编码斜杠/点段和非法编码；独立 Status 域名根仍允许。 / Reject normalized marketing-home aliases while retaining an independent status site's root. | `3ceb5cf2e`、`44bde4d23` |
| V-F02 P2 邀请码认证交接 | 方案 CTA 与已接受的 legacy 入口先进入现有 `/login`，顶层保留 `code`，`return_to` 固定为已配置完整目标、覆盖访客外层返回地址。真实 `siteHref`、SiteLink、appRouter validator/auth-layout gate、Login/Register/AuthFlow 覆盖双语匿名入口→登录→注册预填/实际提交邀请码→登录成功可信目标；已登录 gate 也覆盖。普通登录、dashboard 默认落点与安全校验实现无改动。 / Preserve top-level invitation and the selected target through the existing trusted auth flow. | `4dcb10f45` |
| V-F03 P2 bundle 门禁 | 检测双引号、单引号及无插值反引号完整 marker，匹配成对定界符；三组件均覆盖全关及每次只开其中一项；前缀/插值模板不误判为完整静态 marker。此门禁仍是静态 marker 检测，并非任意 JS 语义分析。 / Cover all three literal forms and independent switches in the actual build gate. | `714eaabd2` |
| V-O01 P3 非阻塞观察 | 收窄源码声明为剔除关闭控件和请求实现；纯 Zod schema 初始化可能残留，不承诺 adapter 相关字节归零。首轮包未发现可操作 UI 泄漏，门禁缺陷不被描述成已发生泄漏。 / Narrow the exclusion claim; pure schema initialization can remain. | `714eaabd2` |

源码变更仅针对这些边界。`router.tsx`、`app-entry.tsx`、Login/Register/AuthFlow 业务实现、文章发布清单与 Blog 规则均无改动。Blog 仍仅顶部/移动菜单默认隐藏，Footer/直达页面/SEO 保留；另外三功能和九方案激活仍默认关闭。

Changes are limited to the listed boundaries. Existing router/auth implementations, publication policy and Blog behavior remain unchanged. Blog is hidden only in the top/mobile menus; its footer, direct routes and SEO remain. Other optional features and all solution activations still default off.

## 实际验证与历史证据 / Verification and historical evidence

本地串行测试：Status 新反例先失败后通过（16 项）；方案与认证相关 8 文件/79 项通过，补充 legacy 回归后 2 文件/15 项通过；bundle 反引号反例先失败后通过（6 项）；TypeScript `tsc -b --noEmit` 通过。后续完整构建、产物门禁与浏览器只在隔离 CI 单 worker 执行。未在本地运行完整构建/浏览器矩阵或访问 4180，也未启停现有服务。

Local targeted tests reproduced failures before fixes and then passed; serial tests and TypeScript passed. Complete production build and browser checks run only in isolated CI. No local full build/browser matrix, port-4180 access or existing-service changes were performed.

首轮原始 [FAIL 报告](website-visibility-evidence/round1-independent-failure/report.md)、[findings](website-visibility-evidence/round1-independent-failure/findings.json)、[checks](website-visibility-evidence/round1-independent-failure/checks.json)、[探针源码](website-visibility-evidence/round1-independent-failure/independent-probe.cjs)与[反例结果](website-visibility-evidence/round1-independent-failure/independent-probe.json)逐字归档，继续绑定旧 SHA。旧 `verification.json`、`ci-proof.txt` 不被覆盖；新 CI 证据另存 `round1-fixes-verification.json` 和 `round1-fixes-ci-proof.txt`。

The original independent FAIL evidence is archived verbatim and remains bound to its original SHA. Old verification/log files are retained; repair CI evidence uses separate files.

最终 [CI run 39582397033](https://github.com/aelf-hzz780/NyxID/actions/runs/39582397033) **success**，工程 SHA `44bde4d23ded1cba2d7747e00bc146d5feaf0c96`：31 文件 / **241 项测试**、完整 TypeScript/生产构建、90 页产物门禁、219 项 HTTP、日志隐私、10 项单 worker 浏览器全部通过（0 skipped / flaky / unexpected）。中间工程 `714eaabd2` 的 run `39582245022` 因归一化补丁被新 run 取代并 cancelled，不作为最终通过证据。

The final linked CI succeeded for the exact engineering SHA: 241 tests in 31 files, complete TypeScript/production build, 90-page artifact gate, 219 HTTP checks, log privacy and ten single-worker browser tests without skips/flakes. The superseded intermediate run was cancelled and is not final evidence.

[完整 artifact 12337521717](https://github.com/aelf-hzz780/NyxID/actions/runs/39582397033/artifacts/12337521717) 已下载，ZIP SHA256 与 GitHub 完全一致：`sha256:1e86ffc0fdc0f38699622de5734d1ca5e49a5bccf8bc9c7c424f5efbdbf9354c`。下载包 `sourceCommit` 与工程 SHA 一致，重新计算配置和内容摘要通过：

- `contentRevision`: `49e686770de9ed0f5a924923048c88611956e8d8ae7af93bb73744fa9ba236b1`
- `configRevision`: `a27cd071131edbef9665b638f622bf1a541f90673ee733d077c77f71e2faef27`

有效配置仍为 `blogMenu=false, newsletter=null, contactChat=null, footerStatus=null, solutions={}`。摘要和旧默认关闭版本相同，不代表工程源码相同；工程由 `sourceCommit` 单独标识。新机器证据见 [round1-fixes-verification.json](website-visibility-evidence/round1-fixes-verification.json)，CI 日志摘录见 [round1-fixes-ci-proof.txt](website-visibility-evidence/round1-fixes-ci-proof.txt)。

The artifact was downloaded and its ZIP digest matched GitHub. Its source matches the engineering SHA, and content/configuration hashes were independently recomputed. Effective configuration remains all-off. Matching content/configuration revisions do not imply identical source; `sourceCommit` identifies the implementation separately. Machine-readable evidence and log excerpts are linked above.

本次 `/security` 单次 reduced-motion CI lab：LCP 116 ms、CLS≈1.522、73 resources、559985 transfer bytes。仅记录观察，CLS 仍需另行排查，不判性能、field CWV 或本增量回归通过/失败。

The single reduced-motion CI lab sample records LCP 116 ms and CLS≈1.522. High CLS remains a separate concern; no performance, field-CWV or increment-regression conclusion is made.

## 仍未执行 / Remaining unexecuted scope

- 第 2 轮正式独立复验，及整包 HTML/JS 混装浏览器测试。 / Formal round-two independent acceptance and whole-artifact mixed-configuration browser testing.
- on 配置完整生产环境与真实 Newsletter 确认/送达/退订/滥用控制、公共 Chat 配额/限流、真实 Status 可用性验收；当前只有 fixtures。 / Enabled production configurations and live-service acceptance; fixtures establish wiring only.
- 九方案任一真实完整任务、真实账号/OAuth/MFA 端到端；测试中的 target 参数不表示工作区已经支持相应业务。 / Operational solution tasks and live account/OAuth/MFA flows; fixture targets do not establish task support.
- 新文章批准、部署、merge、组织 PR 与 field CWV。 / Article approval, deployment, merge, organization PR and field CWV.

恢复/回滚继续按[原交付步骤](website-v4.2-visibility-delivery.md)：真实服务/目标完整验收后配置对应单项、完整重建并发布同一产物；关闭对应开关或移除单个方案配置后重建回滚。主对话的三份视觉材料保持未编辑、未暂存。

Restoration/rollback follows the existing handoff: accept the actual service/target, configure the individual item and rebuild/release a complete artifact; disable only that item and rebuild to roll back. The main chat's three visual materials remain untouched and unstaged.
