# 首轮验收修复证据 / First acceptance fix evidence

日期：2026-10-15。修复基线：`22c5df89fea2876ad7192af1273b3be704e82ba0`。首轮独立验收发现三项缺陷，本记录为实现方定向验证，指定验收对话已于 2026-10-15 对修复 commit `1ac8a28675603279499fb1205790ca4bd67c4fa0` 独立复验通过。

Date: October 15, 2026. Fix base: `22c5df89fea2876ad7192af1273b3be704e82ba0`. These are implementation checks for the three independent acceptance findings, followed by independent re-acceptance, which passed on commit `1ac8a28675603279499fb1205790ca4bd67c4fa0` on October 15, 2026.

| Finding | Fix and actual verification |
| --- | --- |
| A-01 / P2 | Generator adds a mobile-only login link inside the expanded menu; existing auth helpers preserve the full query. At actual **390×844**, both locales expose a visible, focusable link (`tabIndex=0`). Pressing Enter navigates to real `/login` with `code`, nested `return_to`, UTM and language unchanged. At 1280px, only the original desktop login is visible and the burger is hidden; no horizontal overflow. See `browser-audit.json`, `mobile-menu-en.jpg`, `mobile-menu-zh.jpg`. |
| A-02 / P3 | Generator and both generated Solutions pages restore “Ask NyxBot / 交给 NyxBot”, dashed border, centered alignment and CTA spacing from A. Computed styles confirm these properties. Click/Enter reaches `/nyxbot` with query intact in both locales. See `custom-solution-en.jpg`, `custom-solution-zh.jpg`, `browser-audit.json`. |
| A-03 / P3 | Restore original local 24×24 icon, 7px radius, badge 10px font, 1px/6px padding and auto left margin. All four demo selections in both languages were measured at 1280×900, including Restaurant manager: all eight active badges end inside their row (9px inset in this run). See `sidebar-en.jpg`, `sidebar-zh.jpg`, `browser-audit.json`. No global icon/badge CSS changes. |

新增双语生成内容回归测试在修改前四项均失败，修改后通过；覆盖菜单内真实登录入口和 query、自定义方案 CTA 与目的地/样式。`regression-before.txt` 为失败摘要；`tests.txt` 为最终 11 文件 / 89 项通过记录。A-03 尺寸与溢出使用真实浏览器几何数据验证，不将 DOM 测试当作布局引擎。

The four new bilingual generated-contract tests failed before the fix and passed afterward. `regression-before.txt` records the failure summary; `tests.txt` records the final 11 files / 89 passing tests. Sidebar layout uses actual browser measurements rather than treating DOM tests as a layout engine.

- `build.txt`: full `npm run build` passed (TypeScript, Vite, legal prerender, credential-accept and mock-footprint checks). Existing chunk-size warnings remain; no performance certification is implied.
- `lint.txt`: `npx eslint src/features/website-a` exited 0.
- `generation.json`: re-running `node scripts/website-a/generate.mjs` reproduced all 68 checked generated/CSS/asset files byte-for-byte.
- Existing static 24-card v4.1 tool wall and all website CSS are unchanged. No backend, auth protocol, billing, new service integration, PR creation, merge or deployment.

复现命令沿用 `website-a-delivery.md`，本次增加的 `src/features/website-a/generated-contract.test.tsx` 已由该测试命令包含。浏览器预览为 `npm run dev -- --host 127.0.0.1 --port 4180 --strictPort`。初始移动端工具限制已解除，不再视作永久阻塞；本次没有重复无关的全量 5625 项测试或 100 组路由检查。

Use the commands in `website-a-delivery.md`; its test selection includes the new generated-contract file. Preview with `npm run dev -- --host 127.0.0.1 --port 4180 --strictPort`. The initial mobile viewport limitation is resolved. The unrelated full 5625-test suite and 100 route combinations were not rerun for these focused fixes.

剩余范围：未执行真实账号登录/OAuth consent/MFA、物理手机触摸/性能、Firefox/WebKit 或完整离线测试。此前验收报告中的相关限制仍成立。独立复验已完成并通过，不扩展为上述未覆盖项的通过声明。

Remaining limits: no real-account login/OAuth consent/MFA, physical-device touch/performance, Firefox/WebKit or full offline run. The earlier acceptance report's corresponding limitations still apply. Independent re-acceptance passed for this delivery scope; it does not certify those untested areas.
