# v4.2 追加授权轮修复交接 / Additional authorized-round repairs

2026-10-21（Asia/Shanghai）。前两轮正式独立验收均 **FAIL**。人类明确选择“追加一轮，修复并复验”，授权超出原两轮限制，且仅追加这一轮。当前记录是工程修复与验证交接，**追加轮正式独立复验尚未执行**。

Both earlier formal independent rounds **failed**. The human explicitly authorized exactly one additional repair/independent-verification round. This document records engineering work; the additional formal independent verification remains pending.

基线 HEAD `f70b7c4c75baa0b77646a9fe986866ff67327128` / 工程 `44bde4d23ded1cba2d7747e00bc146d5feaf0c96`。仓库/分支 `aelf-hzz780/NyxID:codex/website-a-delivery`。

The baseline source and fork branch are identified above. Earlier CI success does not establish acceptance of the remaining paths.

当前工程 SHA：`9c54de6c630b6e32fc9602ba5bbb9acbb0d39c16`。最后交付 HEAD 包含随后追加的文档/证据 Commit；工程至 HEAD 的差异应仅为这些记录。

The engineering SHA is above. The delivery HEAD also includes the subsequent documentation/evidence commit only.

| 发现 / Finding | 实施与实际覆盖 / Implementation and coverage | Commit |
|---|---|---|
| V-F02 P2 真实 `/start` 漏修 | `router.tsx` 的实际 `beforeLoad` 改用 `solutionStartHref`，fragment start/done 共享该 resolver，CTA 的 `solutionAuthPath` 也由它复用。顶层 `code` 保留，`return_to` 固定为完整已配置目标并覆盖访客外层值。直接调用 appRouter 注册的真实 handler，覆盖 assistant/dashboard、Login→Register 邀请预填、未开放方案回详情及无 `t` 通用注册。 / Share the real route's handoff with fragment and CTA behavior; exercise the registered handler and fallbacks. | `c0ba120e8` |
| V-F04 P2 on 浏览器旧断言 | 启用分支解析 URL，逐项检查 `/login` 和解码后的完整 `return_to`。保留 disabled 分支。新增单项 on fixture 实际运行该分支及 CTA、fragment、真实 `/start`→Login→Register 顶层邀请码/目标的浏览器回归。 / Parse the auth URL contract and execute the enabled branch plus all three entry routes in one targeted fixture. | `16a70cefe` |
| 只读审查发现的 fixture 阻断 | 测试服务器的 public config 从 `{}` 补为符合 `PublicConfig` 的必填字段，空 social providers、邀请码必需、email auth 开启，无遥测设置。避免真实 AuthFlow 读取缺失数组崩溃；不修改认证业务行为。 / Supply the fixture's valid public-config contract for actual auth pages. | `9c54de6c6` |

默认交付包四开关仍 false、solutions 空。Blog 仅顶部/移动菜单隐藏，Footer/直达页/SEO 保留。V-F01、V-F03 及 V-O01 的已关闭结论无需扩大重做；本次未改 Status、Contact、Newsletter 或 bundle marker 实现。认证保护、普通登录/dashboard 默认落点、可信 `return_to` 校验实现不变，仅修复真实 `/start` 的交接。

The default delivery stays all-off. Blog hiding remains menu-only. Previously closed findings are not reopened. Existing auth guards, ordinary login/dashboard defaults and trusted-return validation remain unchanged; only the real `/start` handoff is corrected.

## 验证设计与执行边界 / Verification and scope

本地新增真实 handler 反例先失败（2 fail / 5 pass），修复后相关 6 文件 / 53 项测试通过；`tsc -b --noEmit` 通过。没有本地完整构建、浏览器矩阵、IAB/4180 或现有服务启停。

The real-handler regression failed before the fix and passed afterward; targeted serial tests and TypeScript passed locally. No local full build/browser matrix, port-4180/IAB access or existing-service changes occurred.

隔离 CI 的默认 `public-artifact` job 保留完整回归和默认发布包。新增 `solution-on-fixture` **依赖默认 job 成功后顺序执行**，完整构建唯一 help-desk on 配置，并运行产物门禁及配置断言；随后只运行两个定向浏览器测试，单 worker / 无 retry。额外断言要求 exactly 2 expected、0 skipped/flaky/unexpected，避免 on 分支未执行仍报绿。

The isolated on-fixture job runs only after the default job succeeds. It builds exactly one enabled solution, verifies its configuration and artifact gates, then runs two targeted single-worker browser tests with no retries or skipped/flaky/unexpected results. This is not a full feature/viewport matrix.

fixture 的 help-desk target 为 `/assistant?setup=help-desk&draft=hello%20world`，配置解析后空格规范为 `+`；非敏感 acceptance 标为 `ci/wiring-fixture-only`。四个展示开关仍 false，其余八方案不启用。`solution-fixture.json` 明确 `productionDelivery=false`、`realTaskAccepted=false`，包名为 `website-solution-fixture-*`，不能替换默认交付包 `website-public-*`。

The fixture target and acceptance reference are synthetic wiring inputs. Its metadata and distinct artifact name explicitly prohibit treating it as production delivery or a completed real solution task. All other feature switches and solutions remain off.

## 实际 CI 与产物 / Actual CI and artifacts

[最终 run 39610188716](https://github.com/aelf-hzz780/NyxID/actions/runs/39610188716) **success**，两份产物的 `sourceCommit` 均为 `9c54de6c630b6e32fc9602ba5bbb9acbb0d39c16`。默认 job（`119257290465`）31 文件 / **245 单测**、完整构建、90 页门禁、219 HTTP、隐私门禁、**10 browser** 通过。随后 on job（`119259294390`）完整构建与90页门禁通过，实际运行 **2 browser**，包含更新的 enabled 断言和三种入口→Login/Register。两组均 0 skipped / flaky / unexpected。

The final linked run succeeded. Both artifacts identify the exact source SHA. The default job passed 245 tests, complete build, 90-page and 219-HTTP/privacy gates, and ten browser tests. The dependent on-fixture job passed its complete build/90-page gate and two targeted browser tests, including the enabled branch and all three real entry routes. Neither group had skipped, flaky or unexpected results.

| 产物 / Artifact | contentRevision | configRevision |
|---|---|---|
| [默认交付 12350049039](https://github.com/aelf-hzz780/NyxID/actions/runs/39610188716/artifacts/12350049039) | `49e686770de9ed0f5a924923048c88611956e8d8ae7af93bb73744fa9ba236b1` | `a27cd071131edbef9665b638f622bf1a541f90673ee733d077c77f71e2faef27` |
| [仅接线 fixture 12350586533](https://github.com/aelf-hzz780/NyxID/actions/runs/39610188716/artifacts/12350586533) | `36fef752a19c00ab60bc6b9e2a59c0f320d4d7bb257b7b3f4e0f505694d93b5b` | `dae178b5d7afc1c3e71fc7b485ca3f7ac1002a4e432f40e942a174f595c62839` |

两份 ZIP 均已下载，digest 与 GitHub 一致，重新计算 content/config revision 并核对配置、源码 SHA、浏览器结果通过：默认 `sha256:ea3d31433143004a79eefb6f8e338f5df31f7db413e5deb9195e0f8cb0c2586b`；on fixture `sha256:04d43e2dcad8f63e75c4f997dee37e5d7632cb0c1ad514ab2fea251c90cff634`。两份配置及摘要不同，不能混用。中间 run `39609940633` 被 fixture 配置修正后的最终 run 取代并 cancelled，不作为通过证据。

Both ZIP files were downloaded and their digests matched GitHub; source/configuration, recomputed revisions and browser statistics were verified. Their differing configurations/revisions must not be mixed. The superseded intermediate run was cancelled and is not final evidence.

机器证据：[extra-round-verification.json](website-visibility-evidence/extra-round-verification.json)；原始日志摘录：[默认](website-visibility-evidence/extra-round-off-ci-proof.txt)、[on fixture](website-visibility-evidence/extra-round-on-ci-proof.txt)。默认包单次 reduced-motion lab：LCP 436 ms、CLS≈1.948、73 resources、560301 transfer bytes；仅记录观察，高 CLS 仍需另行排查，不判性能或 field CWV 通过/失败。

Machine evidence and log excerpts are linked above. The single default-package lab sample is informational only; high CLS remains a separate concern, with no performance/field-CWV or increment-regression conclusion.

## 历史证据与未执行项 / Historical evidence and unexecuted work

首轮 [FAIL 原件归档](website-visibility-evidence/round1-independent-failure/report.md)与第二轮 [FAIL 原件归档](website-visibility-evidence/round2-independent-failure/report.md)均保留；第二轮 report/findings/checks/probe 源码和结果按字节复制，未修改为通过。历史交接与 CI 证据继续绑定旧 SHA。

Both FAIL archives are retained verbatim. Earlier handoffs and CI evidence remain tied to their original SHAs.

仍未执行：追加轮正式独立复验；真实账号/OAuth/MFA；真实 Newsletter 确认/送达/退订及滥用控制、公共 Chat 配额/限流、Status 可用性；真实方案配置/授权/任务完成；其它功能 on 或全视口矩阵、整包混装浏览器测试、新文章批准、部署/merge/组织 PR、field CWV。fixture 仅证明链接和认证接线，不能替代这些验收。

Unexecuted work includes formal independent verification, live auth/services and operational tasks, broader enabled/viewport matrices, mixed-artifact browser testing, article approval, deployment/merge/organization PR and field CWV. Fixtures establish wiring only.

恢复/回滚仍按[原交付说明](website-v4.2-visibility-delivery.md)：真实验收后单项配置、完整重建发布；关闭单项或移除方案配置后重建回滚。主对话三份视觉文件与旧未跟踪资产不编辑、不暂存。

Existing individual enablement/rebuild and disablement/rebuild rollback instructions remain. The main chat's visual materials and older untracked assets are untouched and unstaged.
