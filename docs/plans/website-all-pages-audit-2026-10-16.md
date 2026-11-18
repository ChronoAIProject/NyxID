# 官网逐页自查 / Website page audit

本轮覆盖当前官网 A + v4.1 的 25 条公开页面路径、每页中英文、站内跳转和现有认证入口。页面与站内链接检查通过，但订阅和品牌水印仍是明确未完成项，不能据此宣布整体上线验收通过。没有在 Codex 内打开页面，也没有运行图形浏览器自动化。

This audit covers all 25 public website routes in English and Chinese, internal links and existing authentication entries. Page/link checks passed; newsletter integration and scene branding remain open. This is not overall launch acceptance. No in-app or graphical browser automation ran.

## 覆盖结果 / Coverage

| 页面 / Pages | 数量 / Count | 证据 / Evidence |
| --- | --- | --- |
| Home、NyxBot、Solutions、How it works、Security、Agents、Pricing、Integrations、Blog、Contact | 10 × 2 languages | 实际 React 页面渲染、标题/主内容/导航/页脚、站内目标和查询参数 / Actual component rendering, structure, destinations and query preservation |
| Solutions details | 9 × 2 | 每个详情可渲染，相关入口有效 / Every detail renders with valid internal destinations |
| Blog articles | 6 × 2 | 每篇文章可渲染，相关入口有效 / Every article renders with valid internal destinations |
| Auth、legal、Docs 与文档深链 / Auth, legal and Docs | 9 HTTP checks | 加上上述 50 页，共 59 次 GET 均 HTTP 200 / 59 successful HTTP checks including the 50 localized routes |

新增页面矩阵使用实际 `WebsitePage` 与页面组件，使用 happy-dom 并模拟 location/motion API，避免 GPU、真实浏览器和页面网络请求。筛选样例覆盖 Solutions 分类与渠道、Integrations 搜索/零结果/恢复、Blog 分类、语言切换，以及顶部和详情的栏目选中。现有路由注册、官网交互、Login/Register、AuthFlow 和可信回跳测试一并串行执行：**15 个文件、199 项通过，单 worker，约 26.58 秒**。相关 7 文件 ESLint、语法与 Git whitespace 检查通过。

The page matrix renders real website components in happy-dom with location/motion APIs stubbed. It checks sample filters, language changes and top-level/detail navigation markers. Together with route registration, website interactions, Login/Register, AuthFlow and trusted return URL tests, **199 tests across 15 files passed in one serial worker**. Scoped lint, syntax and whitespace checks also passed. This does not establish pixel fidelity, real browser history behavior or every external destination's availability.

生成器在隔离临时目录重新运行，**68 个输出与当前工作树逐字节一致**。未修改产品原稿，未覆盖当前工作树。独立验收对话另逐页对照原型，除本轮发现并修复的导航高亮外，未发现新的整块内容或结构删减。

Isolated regeneration reproduced **68 files byte-for-byte** without modifying the prototype or working tree. The independent review compared pages against the prototype and found no additional wholesale content/structure omissions beyond the navigation marker fixed in this round.

## 已修复的问题 / Corrections

1. **本地认证配置 502**：恢复 4180 时漏传线上 API 配置，导致兜底登录设置与线上不同。现已恢复单个服务并验证公开配置一致；启动命令见最新交付说明。
2. **旧式认证链接丢参数**：`#/login`、`#/signup` 现在合并外层查询与片段中的邀请码、可信回跳、语言等参数。目标参数优先，路径白名单不变；`#/start`、`#/done` 不传入原型 provisioning 状态。
3. **顶部导航缺少当前栏目样式**：统一完整路径段匹配。顶级页面标记 `aria-current=page`，Solutions/Blog 详情标记所属栏目 `aria-current=location`；生成器与双语 Header 同步。

Three issues were corrected: missing production API settings in the local preview; dropped authentication parameters in legacy fragment links; and missing active navigation styles/parent-section markers. Existing auth code and security boundaries remain intact.

## 认证与线上一致性 / Authentication parity

通过只读公开接口核对：当前线上版本 `0.71.0`，启用 GitHub、Google、Apple，`invite_code_required=false`、`email_auth_enabled=false`。本地 4180 代理返回同样的值。三个社交登录起点分别返回 302 到 `github.com`、`accounts.google.com`、`appleid.apple.com`，OAuth callback 都指向正式 `nyx-api.chrono-ai.fun`。没有跟随授权、登录真实账号或提交授权同意。

Public production configuration and the local proxy agree on version `0.71.0`, the three enabled providers, no required invitation and disabled email/password authentication. Each social start returned a 302 to its official provider with the production callback. No authorization redirect was followed and no account login or consent was performed.

`LoginPage`、`RegisterPage`、AuthFlow、MFA、`use-auth` 和可信回跳核心文件与 `upstream/main` 无差异；线上公开登录/注册模块亦保留 `return_to`、邀请码与 MFA 接线。既有 AuthFlow 在内部切换登录/注册面板时只保留 `code`、`return_to`，本轮按线上行为保留，不能将官网入口“保留完整查询”扩张为面板内部也保留所有营销参数。

Core authentication files are unchanged against `upstream/main`; public deployed entry modules retain return/invitation/MFA wiring. Existing in-panel login/register switching preserves only `code` and `return_to`, matching production. Website-entry query preservation must not be described as preservation of every campaign parameter throughout the auth UI.

## 明确边界 / Remaining limits

- Blog 订阅仍没有接收服务，按钮明确未开放；没有伪造订阅成功。
- 首屏 UnicornStudio 品牌水印仍需解决。
- Contact 使用 `contact@chrono-ai.fun` 的邮件客户端交接，需在客户端实际发送；本轮未测试 OS 客户端或邮件投递。
- 原型演示、静态工具目录和示例权限不是实际服务调用；没有接入匿名免费 AI。
- 本轮没有真实账号 OAuth/MFA 完整认证、图形浏览器逐页视觉、物理设备或性能指标验收。

Newsletter integration and scene branding remain open. Contact opens a draft for user-controlled sending. Scripted demonstrations and static catalogue content are not service execution. Real-account OAuth/MFA completion, OS email delivery, graphical/physical-device appearance and performance metrics remain unverified.

证据 / Evidence: [审计数据 / Audit data](website-v4.1-evidence/full-site-audit.json)、[测试结果 / Tests](website-v4.1-evidence/full-site-tests.txt)、[生成复现 / Regeneration](website-v4.1-evidence/full-site-generation.json)。审计 JSON 记录被测运行文件哈希，避免把旧验收结论冒充当前代码证据。
