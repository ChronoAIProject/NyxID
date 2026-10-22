# v4.2 裁剪评审材料 / Scope review artifacts

[完整方案](../2026-10-16-website-scope-cut-v4.2.md) · [前后对照](review.html) · [证据](evidence.json)

本目录是独立评审材料，不替换 v4.1，不修改正式官网。Before 从 v4.1 视觉候选模板提取；After 只体现六项范围决定和此前已确认的 Contact mailto 行为。所有页面切片均无脚本、iframe、Canvas 或外部资源加载；可在独立浏览器打开。截图所用字体是本机系统回退字体，前后使用相同条件。

This directory contains independent review artifacts, not production changes. Before excerpts come from the v4.1 visual candidate; after excerpts illustrate the six scope decisions and the previously approved Contact mailto behavior. Excerpts contain no scripts, frames, canvas or remote resources and can be opened in an external browser. Both sides use identical local fallback fonts.

对照板包含六项决定、五组桌面截图及两组移动端截图，另附 20 个中英文页面/区域切片。Dashboard 使用行为对照，因为 v4.1 原型没有真实登录态；不会伪造一张“已登录截图”证明路由可用。

The board covers six decisions, five desktop screenshot pairs, two mobile pairs and 20 bilingual page/region excerpts. Dashboard is compared through a behavior diagram because the prototype does not implement real authenticated sessions.

复现命令（仓库根目录执行；使用已有 frontend 依赖）：

Reproduce from the repository root using existing frontend dependencies:

```sh
node docs/plans/website-scope-v4.2/source/build.mjs
python3 docs/plans/website-scope-v4.2/source/build-review.py
node docs/plans/website-scope-v4.2/source/capture.mjs
```

最后一步会短暂运行一个独立 headless Chromium，禁用脚本、GPU 和网络访问，只读取本目录的静态切片，完成即关闭。不会访问 4180、加载原型动画或打开 Codex 内置浏览器。受限环境若无法启动浏览器，不可声称截图已更新，应保留已有截图并记录限制。

The capture step briefly runs one external headless Chromium with JavaScript, GPU and network access disabled. It reads only these static excerpts and closes when complete. It never visits localhost:4180, loads the animated prototype or opens the Codex in-app browser. If browser launch is restricted, do not claim captures were refreshed.

本次检查：68 项静态结构/范围断言、57 个对照板链接及方案相对链接检查；3 个 v4.1 基线文件 SHA-256 保持一致。截图生成后检查了 Contact、禁用 CTA 和 Discord 排版。独立只读复核未发现六项范围及认证边界的实质偏差。上述结果仅验证评审产物，不替代官网实施验收。

Checks performed: 68 structural/scope assertions, 57 board links and proposal-relative links, plus unchanged hashes for three v4.1 baseline files. Captures were inspected for Contact, the disabled CTA and Discord layout. Independent read-only review found no material scope or authentication-boundary discrepancies. These checks validate review artifacts only.

2026-10-21 更新：review.html 与 build-review.py 同步采用默认隐藏、保留源码、配置后单项恢复口径，新增 Blog 主菜单开关。build.mjs 中清除 DOM 的动作仅生成默认关闭的静态评审切片，不删除业务组件。原有截图为历史评审目标，本轮没有重新 capture；不要把它们称为最新运行截图。新的 visibility-review.html 及其生成器/证据由主对话独立维护，本轮不修改或暂存。

Updated 2026-10-21: the original review generator now reflects retained default-off features and the Blog navigation switch. DOM removal in build.mjs represents only the static off-state excerpt, not deletion of business source. Existing screenshots are historical review targets; this revision performs no browser capture. The separate visibility review and its generator/evidence are owned by the main chat and excluded from this commit.
