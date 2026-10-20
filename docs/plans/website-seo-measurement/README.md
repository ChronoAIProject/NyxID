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
