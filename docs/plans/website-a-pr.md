# Website A + v4.1 PR preparation

Delivery uses implementation commits on `aelf-hzz780/NyxID`, branch `codex/website-a-delivery`. The owner creates the organization PR manually. No merge or deployment is authorized here.

- Optional fork comparison: https://github.com/aelf-hzz780/NyxID/compare/codex%2Fwebsite-a-base...codex%2Fwebsite-a-delivery?expand=1
- Organization comparison: https://github.com/ChronoAIProject/NyxID/compare/main...aelf-hzz780:codex%2Fwebsite-a-delivery?expand=1
- Upstream base: `8280098785b477a9980cc8aed98f43789d6fd3dc`.
- Historical A acceptance: `1ac8a28675603279499fb1205790ca4bd67c4fa0`. v4.1 requires new independent acceptance.

## Title

feat(website): integrate approved bilingual website and v4.1 visuals

## Body

Integrate the approved website into 25 bilingual native React routes while preserving real authentication, full query propagation and native showcase/workflow/filter behavior. Add approved GradientText, SpecularButton, SpotlightCard and FadeContent effects with scoped styles, precise placement and lifecycle cleanup. Keep the homepage's original24 static tool cards. Public pages now independently default to the approved dark theme without altering application preferences.

Local validation:125 tests across16 files, full production build, scoped ESLint and deterministic68-file generation passed. Actual browser checks cover desktop/bilingual390px mobile, native Enter/query navigation, shader/gradient/fade output, offscreen recovery, filters and eight NyxBot sidebar states. Lifecycle tests cover reduced motion, visibility and partial renderer/network failures. See `docs/plans/website-v4.1-delivery.md` and evidence for commands and limits.

**Open blocker:** the reused UnicornStudio scene still displays branding. No authorized watermark-free resource was found in the repository. No cover layer, branding DOM removal, SDK modification or purchase was performed. Obtain an authorized unbranded publication or approve a replacement before overall acceptance/merge. Independent v4.1 acceptance is pending; physical devices, cross-browser, real OAuth/MFA and production CWV remain unverified.
