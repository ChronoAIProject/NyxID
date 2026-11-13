# Studio, NyxBot, and CLI wizard languages

The web frontend shares one i18next instance across Studio, NyxBot, the landing
page, NyxBot onboarding, and CLI browser wizards. It supports English (`en`), Simplified Chinese
(`zh-CN`), and Traditional Chinese (`zh-TW`).

## Choosing a language

Open **Settings → Profile → Language** in Studio or NyxBot. A selection updates mounted translated components immediately;
switching language preserves a chat draft and does not submit it. The frontend
also updates the document's `lang` attribute.

The choice is saved under `nyxid.language` in browser local storage and shared
between tabs on the same origin. It is browser-local; it is not an account
setting and does not synchronize between devices. When storage is unavailable,
switching still works for the current page.

At startup the frontend uses, in order:

1. A supported saved language.
2. The previous `nyxbot-onboarding-language` preference, if present.
3. The first supported browser language preference.
4. English.

Chinese locale tags resolve by script. Explicit `Hans` selects Simplified Chinese
and `Hant` selects Traditional Chinese. Regional defaults include CN/SG as
Simplified and TW/HK/MO as Traditional. English variants resolve to `en`.

## Current translated surfaces

The shared language preference covers first-party web interface copy:

- Studio navigation, command search, breadcrumbs, dashboard, services, providers,
  credentials, agent keys, organizations, devices, channels, machines, billing,
  usage reporting, account settings, and admin pages, including their forms,
  tables, dialogs, and status labels.
- NyxBot navigation, home, account panels, settings, agent and group forms,
  details, access controls, chat composer, attachments, message controls, action
  and approval cards, authored-skill review, and decision receipts.
- Sign-in, registration, device and Agent Key authorization, OAuth consent,
  credential connection, MFA, and account security screens.
- NyxBot onboarding in all three languages, including the new Traditional
  Chinese catalog. The landing page retains its three existing locale catalogs.
- Local CLI browser wizards and hosted `/cli/pair` pages: service setup, API keys,
  node tokens, service accounts, developer apps, MFA enrollment and recovery
  codes, permission pickers, validation, and completion/recovery screens. Each
  wizard has a segmented language control at the right of its header: EN for
  English, CN for Simplified Chinese, and ZH for Traditional Chinese. The active
  language has a raised accent background. Arrow keys, Home, and End change languages.
- Shared controls, frontend validation, dates, numbers, and relative times using
  the selected locale. Existing schema errors revalidate when language changes;
  drafts, dirty state, and manually supplied server errors are preserved.

Known NyxID API errors, authentication/device-login failures, network failures,
and action-failure fallbacks translate at presentation time. The shared error
catalog covers Studio, NyxBot, inline validation, toasts, and browser wizards.
Raw `ApiError.message`, response bodies, numeric codes, and retry decisions are
preserved. Standard NyxID error prefixes translate while unrecognized details,
resource names, identifiers, and provider messages remain verbatim.

Unrecognized server/provider error details, emails, channel notification bodies, terminal CLI
output are outside the frontend change. The native integration is described below. Custom names,
descriptions, chat content, provider output, and authored documentation, blog,
and legal document content remain in their authored language. Their web controls
use the interface language.

The interface preference does not set an LLM response-language override. NyxBot
continues to follow its existing instructions to respond in the user's language.

The main app downloads each large Chinese Studio/NyxBot catalog only when that
language is selected. Initial rendering waits up to 1.5 seconds for the selected
catalog, then permits English fallback while a slow download finishes. Transient
download failures retry; mounted controls update when the catalog arrives. Later
language changes wait for their catalog without losing drafts. English phrases
serve as fallback keys, with only plural and other distinct English values
included in the runtime overrides. The smaller shared setup, landing, and
onboarding catalogs remain bundled. Loading these by route is a future
optimization. The standalone CLI wizard loads only common formatting, error,
and wizard catalogs.

Language preferences are per origin. Hosted pairing shares the Studio/Bot
preference. Local CLI wizards use ephemeral loopback ports, so a saved selection
is not guaranteed to carry across separate runs on different ports; each run
also detects the browser language and offers the header selector.

## Adding translations

`frontend/src/i18n/instance.ts` owns the shared instance, preference listeners,
and formatting bridge. `index.ts` registers app catalogs; `wizard.ts` registers
the wizard catalogs without importing the main app.

| Namespace | Resources | Callers |
| --- | --- | --- |
| `common` | `src/i18n/locales/common-{en,zh-CN,zh-TW}.json` | Shared date/relative-time formatting |
| `errors` | `src/i18n/locales/errors-{en,zh-CN,zh-TW}.json` | Shared known error messages and error-envelope templates |
| `wizard` | `src/i18n/locales/wizard-{en,zh-CN,zh-TW}.json` | Local browser wizard, hosted pairing, shared setup controls |
| `app` | `src/i18n/locales/{en,zh-CN,zh-TW}.json` | Studio and NyxBot UI |
| `translation` | `src/features/landing/i18n/locales/` | Landing page (default namespace) |
| `onboarding` | `src/features/nyxbot-onboarding/locales/` | Onboarding provider with `defaultNS="onboarding"` |

Use the app hook in a component so it re-renders after a language change:

```tsx
import { useAppTranslation } from "@/i18n/use-app-translation";

function Greeting({ name }: { name: string }) {
  const { t } = useAppTranslation();
  return <h1>{t("Welcome back, {{name}}", { name })}</h1>;
}
```

App keys are complete English phrases, stored as flat JSON keys. Add the same
key to all three catalogs, preserving interpolation placeholders. Use `count`
with `_one` and `_other` English variants when a phrase needs plural forms.
Interpolate whole phrases instead of joining translated sentence fragments.
Missing app translations fall back to English, then the source phrase.
Run `npm run sync:i18n` after changing distinct English values or plurals to
refresh `en-overrides.json`. Production builds also refresh this generated file;
the copy audit rejects stale overrides.

For a helper called during rendering or an event, import `translate` from
`@/i18n`. Do not evaluate it once at module initialization: that would freeze a
label in the startup language. Render callers must still subscribe through
`useAppTranslation`; memoized computations must depend on `t` or the language.

Shared controls use `useUiTranslation` / `translateUi` from
`@/i18n/use-ui-translation`. These use the namespaces registered by the entry
point and keep the standalone wizard independent of Studio catalogs. Add any
new shared control copy to the wizard catalogs when that control is in the
wizard's source closure.

Shared formatting helpers use the dependency-free `lib/formatting-locale.ts`
bridge, configured by the shared i18n instance at startup and on language changes.
Standalone callers that never initialize i18n retain English defaults. Wizard
components use `useWizardTranslation` (or `translateWizard` for helpers) from
`@/i18n/wizard`; keep their imports clear of app/landing/onboarding catalogs.
Error toasts use `translateErrorMessage` from `lib/error-messages.ts`. It looks
up the error catalog and translates known envelope prefixes without changing
the original error object. Add new app-owned failure messages to all three
error catalogs. Keep behavior comparisons on raw errors and stable codes.

After changing a file in the wizard's source closure, rebuild its checked-in assets:

```sh
npm --prefix frontend run build:wizard
cargo test -p nyxid-cli --test wizard_bundle_freshness
```

Translate app-owned copy only. Keep user data, resource names, URLs, credentials,
request values, enum values, keyboard key names, and protocol identifiers intact.
Translate the visible label for a value separately from the stored value. React
escapes interpolated text; do not render translations as raw HTML.

## Validation

From `frontend/`:

```sh
npx tsc -b --pretty false
npm run check:i18n
npm test -- src/i18n src/components/settings/language-settings.test.tsx src/lib/utils.test.ts src/pages/nyxbot-onboarding.test.tsx
npm run test:e2e -- e2e/multilingual.spec.ts e2e/wizard-scope.spec.ts --workers=1
```

The copy audit checks catalog parity, interpolation, product terminology,
static translation keys (including conditional keys), and direct visible JSX
copy. It rejects date and number formatting that implicitly uses the browser's
locale instead of the selected interface locale. It also prevents translation of protocol examples, comparisons, React
keys, and input values. It does not prove arbitrary dynamic copy is localized;
changes to metadata and helper-produced labels still need a render review.
`src/i18n/protocol-examples.json` and `glossary.json` record the deliberate
boundaries for technical examples and product names.
`domain-glossary.json` rejects known wrong meanings for permission grants,
secrets, slugs, routes, agents, callbacks, and pagination cursors. The
`simplified-only.json` character list checks the Traditional Chinese catalogs;
it is derived from OpenCC's STCharacters data, excluding valid Traditional
variants such as 群 and 秘. Script checks do not replace a language review.
Regenerate incorrect Traditional Chinese entries from their English source,
then review Taiwan terminology; do not use the Simplified Chinese catalog as
the translation source.

Catalog tests check key parity and interpolation placeholders. Language tests
cover browser negotiation, explicit scripts, legacy preferences, and unavailable
storage. Component and browser tests cover live switching, shared state, draft
preservation, navigation, persistence, and narrow mobile layouts. Browser tests
use the existing HTTP fixture boundary and do not call a live backend or model.

## Local sample preview

Start the opt-in Vite preview from the repository root:

```sh
cd frontend
NYXID_MULTILINGUAL_PREVIEW=1 npm run dev -- --mode test --host 127.0.0.1 --port 4612 --strictPort
```

- NyxBot: <http://127.0.0.1:4612/assistant?mock=1>
- Studio: <http://127.0.0.1:4612/dashboard?mock=1>
- Service wizard: <http://127.0.0.1:4612/__preview/wizard>
- API key wizard: <http://127.0.0.1:4612/__preview/wizard?flow=api-key-create>
- Node registration wizard: <http://127.0.0.1:4612/__preview/wizard?flow=node-register-token>
- MFA wizard: <http://127.0.0.1:4612/__preview/wizard?flow=mfa-setup>

These pages use sample data without a backend. The wizard intercepts its API
requests locally and returns example credentials; it does not create live keys,
nodes, or accounts. The preview route is available only when explicitly enabled
on the development server and is absent from production builds.
The sample preview starts at the default 1× text size.

## Native iOS language preference

Open **Settings → Language** to choose English, 简体中文, or 繁體中文. The native
app saves `nyxid.language` in device-local AsyncStorage. It follows the same
script-aware Chinese locale rules as web. On a fresh install it uses the first
supported iPhone language, then English. The native and browser storage are
independent; this is not an account preference.

`mobile/src/i18n/LanguageProvider.tsx` loads the preference before mounting the
navigator and updates subscribed controls without replacing the navigator or
chat drafts. Writes are ordered so rapid changes persist the latest selection.
A failed save leaves the current session usable and exposes a retry in Language.

This native pass translates Settings navigation, account/security controls,
Display, Privacy, Voice and dictation settings, workspace navigation labels,
and dictation controls. Other native feature pages and unrecognized error
messages currently retain English fallback. It does not claim complete native
translation parity with the web. Messages, names, provider voice labels, URLs,
credentials and protocol values are preserved. Legal content stays in its
authored language. App language does not change dictation recognition or the
model's response language.

Native catalogs in `mobile/src/i18n/locales` reuse main's matching app phrases
and include native-specific copy. They are small bundled catalogs and import
no browser runtime. Mark app-owned copy explicitly with `useAppLanguage().t`;
never translate arbitrary text in the generic Text component. Keep both
catalogs and interpolation placeholders in sync.
