# 测量准备 / Measurement preparation

本目录是可复用采集模板，目前没有真实 GSC/Bing、AI 答案、客户咨询或 field CWV 数据。空单元格表示未知，不表示零。只在经过授权的正式上线后采集，不为本工程交付擅自部署、购买测试或提交站长平台。

These are reusable collection templates. No Search Console, Bing, AI-answer, inquiry or field-CWV results have been collected. Blank cells mean unknown. Deployment, purchases and webmaster submissions require their own authorization.

核心指标为来自内容的有效人工咨询；辅助指标为公开页面搜索曝光、点击和正确引用。Guardrails 为错语言/错 canonical、草稿泄露、未知页面 200、登录 query 丢失、方案状态误述和页面性能退化。注册/Discord/邮件点击只算意向，不算交付或邮件发送成功。

The primary outcome is verified inquiries originating from content. Supporting measures are search impressions, clicks and accurate citations. Guardrails cover locale/canonical errors, draft disclosure, soft 404s, lost authentication context, availability misinformation and performance regressions. Clicks are intent, not delivery or successful email sending.

`baseline.csv`：D0 记录发布前完整 28 天；D7/D14 观察，不将不足 28 天窗口与完整基线直接做增长结论；D28 对比等长窗口并注明品牌词/非品牌词、语言、重点方案、设备与国家范围。GSC 和 Bing 分开记录，保留导出证据。咨询由团队自愿标注来源类别并去重，不写姓名、邮箱或表单正文。

For `baseline.csv`, D0 covers the preceding complete 28 days. D7/D14 are observations; D28 compares equal windows. Record brand/non-brand, locale, solution, device and country scope. Keep GSC and Bing separate with export evidence. Inquiry records must be voluntary, deduplicated and free of names, email addresses and message bodies.

`ai-questions.json` 固定 20 个问题，每题有中英版本。每次使用新会话、关闭个人记忆、不粘贴预设答案；记录日期、平台与实际模型标识、地区/登录状态、是否联网、原问题、原回答证据、引用 URL 和精确落地页。先做无引导检索，再另开会话提供 canonical URL 测直接阅读；两组不得合并。每题按事实准确、可用状态准确、权限/交付边界准确、可追溯引用四项各记 0/1；未知或未运行记 null。记录错误类型而不是宣称排名提升。平台/模型属于采样时的运行配置，本文件不固定版本。

The fixed 20-question bilingual sample uses fresh sessions without personal memory or seeded answers. Record timestamp, actual platform/model, region/login, browsing mode, prompt, response evidence and cited landing URLs. Run unguided discovery and direct-URL reading in separate sessions and datasets. Score factual accuracy, availability, permission/delivery boundaries and traceable citations separately as 0/1; unknown/not-run is null. Model selection is runtime configuration.

工程事件复用默认关闭的 consent telemetry；仅允许固定事件名和 manifest 的 `page_id`、`locale`。不采集原始 URL、query、邀请码、claim、return_to、搜索/表单内容。事件为 `public_page_view`、`solution_detail_view`、`docs_link_click`、`contact_mailto_open`、`discord_outbound`、`generic_signup_click`。退出 consent 后既有捕获门禁仍生效。

Engineering events reuse default-off consent telemetry and contain only bounded event names, manifest page IDs and locale. Raw URLs, queries, invitation codes, claims, return targets, searches and form values are excluded. Existing capture gates remain authoritative after consent withdrawal.

性能：CI 单浏览器、reduced-motion 下的 HTTP/交互结果只属于 lab 证据。正式现场数据需按页面/语言/设备采集 LCP、INP、CLS 的足量分布；样本不足必须标记。比较新旧产物压缩 JS/CSS、请求数、长任务和低速网络行为；不以一次 HTTP 200、SSR 字符数或测试通过代替 CWV。

CI HTTP/browser checks are lab evidence. Field LCP, INP and CLS need sufficient page/locale/device samples, with insufficient samples marked explicitly. Compare compressed assets, requests, long tasks and slow-network behavior. HTTP 200, SSR length and passing tests do not establish Core Web Vitals.

## D02 可执行采样契约 / Executable sampling contract

v2 固定 20 题，每题 EN/ZH 两个实际 prompt，共 40 prompts；Q01–Q10 为品牌组，Q11–Q20 为非品牌组，各有 `intent`、`brand_group`。此前无采样数据，v2 替换后半组问题以建立非品牌对照。不能把全部题都提及 NyxID 的旧问题集称为非品牌基线。

Version 2 contains 20 questions and 40 localized prompts: ten brand and ten non-brand questions, with explicit intent and group fields. No prior trials exist; the latter half was revised before sampling to establish an unbranded comparison.

采样前在 `access-register.csv` 填写实际 owner 与证据；目前所有 owner 均未确认。只申请表中最小权限，优先由 owner 导出；不创建账号、不购买平台、不改 DNS。缺权限标记 blocked，并继续有授权的数据源。GSC/Bing 的 D0 各保留最近完整 28 天及此前 28 天；D28 比较等长窗口，D7/D14 只观察。按品牌组、locale、方案、设备、国家分层，缺失值为 unknown，不能填零。

Complete the access register with confirmed owners and evidence first. Owners are currently unconfirmed. Prefer scoped read-only exports; no accounts, purchases or DNS changes are authorized. Preserve the most recent complete 28 days plus the preceding 28 days for both search tools. Use equal-window comparisons and retain the listed segment dimensions; unknown is not zero.

平台 A 必须是可查看来源的搜索型答案平台；平台 B 必须是可查看来源的对话型 AI 平台。选用当时实际可访问的不同平台，记录实际名称、可见模型/版本；不可见写 unknown，不猜测后端配置。每题、每语言、每平台独立新会话重复三次 (`repeat_id=1..3`)，固定登录/地区/搜索与记忆设置。先用 Q01/Q05/Q09/Q12/Q19 的双语版本试采，验证证据可保存再扩展全量；pilot 若条件一致可计入全量，否则单列。完整 discovery 为 20×2×2×3=240 次回答。direct-URL reading 另开会话、另记 mode，绝不合并分母。

Select two actually accessible, source-visible platform categories: a search answer platform and a conversational AI platform. Record actual names and visible versions, using unknown when unavailable. Repeat each localized question three times per platform in new sessions with recorded conditions. Pilot the five named bilingual questions before expanding. A comparable pilot may count toward the 240 discovery trials; otherwise report it separately. Direct-URL reading has a separate mode and denominator.

`ai-trials.csv` 一行一个实际回答，保留原 prompt、时间、版本、条件、回答证据及来源 URL；引用、品牌提及、推荐各记独立 0/1，四项正确性评分各记 0/1，未知记空。引用须有指向目标站点的可追溯来源；仅出现产品名称不是引用，推荐须有明确选择建议。重复来源 URL 不增加样本数。`ai-aggregates.csv` 按品牌组、语言、平台、版本、mode 分层；n 只包括条件完整且完成的回答，k 为该字段为 1 的回答数。各指标使用独立的 `{metric}_n`，只包含该字段已知为 0/1 的回答；空评分退出该指标分母并报告 unknown 数量 (`completed_n - metric_n`)，不得用 completed_n 将未知当作零。blocked/not-run 单列、不进入 n；分母为零时报告 unknown，不能报告 0% 或排名/搜索量提升。

Store one completed answer per trial with original prompt, conditions, response evidence and source URLs. Citation, mention and recommendation are separate binary outcomes. Four accuracy dimensions are separately scored; unknown remains blank. A citation requires a traceable target-site source, while a mention alone does not. Aggregate k/n by group, locale, platform, version and mode. Each metric uses its own scored denominator (`metric_n`); blank scores are excluded and unknown counts equal `completed_n - metric_n`. Never divide a partially scored metric by all completed trials. Incomplete, blocked and unrun trials do not enter n; duplicate source links do not create extra trials. A zero denominator is unknown, not 0%.

`crawler-baseline.csv` 收集授权的连续 7–14 天 route 聚合。优先用官方 IP 范围或正反向 DNS 验证并记录证据；仅 UA 匹配归入 unverified，不能混入 verified。保存规范路径/语言、请求/bytes、延迟样本数与 p50/p95、challenge/429/5xx、release_id 与 request_id 是否可用；无权获取则 blocked。不导出 query、bearer、表单正文或原始 referrer。

Collect authorized crawler route aggregates for 7–14 consecutive days. Verify identities with official IP ranges or forward/reverse DNS evidence; UA-only matches remain unverified. Retain route, locale, counts, bytes, latency sample size and percentiles, challenges, 429/5xx and available release/request identifiers. Do not export sensitive request context.
