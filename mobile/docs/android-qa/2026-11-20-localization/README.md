# Chinese billing and voice copy: #2582

Base: `upstream/main` at `b6d194f99`. Verified on 2026-11-20.

The latest QA report lists five allowance labels still shown in English. These
are generated from API metric IDs in `balanceSummary`, so they bypass the normal
translation resources. Voice tariffs have a second gap: translated sentences
interpolate English `credits per …` and `plus … per …` fragments.

The fix gives all 18 current billing metrics localized display names, and
translates each tariff fragment before interpolation. It retains metric IDs,
service/provider/model identities and exact decimal rates. Unknown future metric
IDs retain their readable identifier fallback. Simplified and Traditional Chinese
resources both cover the new display copy.

Other examples in the original issue, including Push Notifications, Request
approval for, Maximum composer height and Unable to start, already use translated
resources on this base. This change addresses the remaining generated labels and
composed tariff text; it does not claim a fresh audit of every application screen.

## Runtime evidence

An Android API 34 ARM64 emulator ran the real `LanguageProvider`,
`BillingWalletCard`/settings rows and `voiceTariff` formatter in an Expo Go SDK 55
fixture. The fixture uses the actual billing response schemas and mocked
allowances; it does not sign in or change a real account.

Switching the fixture through its on-screen language controls produced:

| Metric | Simplified Chinese | Traditional Chinese |
| --- | --- | --- |
| input_tokens | 输入词元 | 輸入詞元 |
| cache_write_tokens | 缓存写入词元 | 快取寫入詞元 |
| output_tokens | 输出词元 | 輸出詞元 |
| cache_read_tokens | 缓存读取词元 | 快取讀取詞元 |
| images | 图像 | 影像 |

UIAutomator assertions found all five localized labels in the rendered native
tree. The visible voice paragraph contained both exact fixture rates
`0.000000000001` and `0.000000000002` and no `credits per` or English metric
fragments. The service identity `OpenAI GPT-4o` stayed unchanged. Screenshots and
the extracted visible text are saved in [zh-CN.png](zh-CN.png),
[zh-CN.json](zh-CN.json), [zh-TW.png](zh-TW.png) and [zh-TW.json](zh-TW.json).

This verifies native rendering of the affected components and live language
switching. The reported complete Xiaomi/MIUI QA build was not installed or
modified, and no full-app device sweep was performed.

## Focused local checks

From `mobile/`:

```sh
pnpm sync:i18n
pnpm exec tsx --test src/features/billing/billingMetric.test.ts src/features/billing/balanceSummary.test.ts src/features/billing/BillingPresentation.test.ts src/features/assistant/voice/voiceSelection.test.ts src/features/account/VoiceSettingsScreen.test.ts
pnpm exec jest --findRelatedTests src/features/billing/billingMetric.ts src/features/billing/balanceSummary.ts src/features/assistant/voice/voiceSelection.ts --runInBand --passWithNoTests
```

- Catalog synchronization passed and regenerated the existing area barrels.
- All 42 explicit related node tests passed, including locale changes, both
  credential classes, exact prices, applicability/sync filters and unknown IDs.
- The Jest dependency query found no matching Jest tests; corresponding node
  tests ran explicitly.
- Changed-file TypeScript transpilation reported no syntax errors, and
  `git diff --check` passed. There is no affected mobile typecheck target or lint
  script. Full frontend tests, typecheck and production build are delegated to
  GitHub CI under the personal local-validation policy.

CI follow-up: the catalog coverage guard rejected two newly added phrase keys
with leading spaces. The keys now contain no edge whitespace; sentence templates
and joining own the visible separators. After the correction,
`pnpm exec tsx --test src/i18n/i18nCoverage.test.ts` passed all 12 owning tests,
including catalog interpolation, authored-key coverage and translation composition.
