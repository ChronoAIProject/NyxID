# NyxBot face kit, visual styles and process emotes

The renderer decision in this historical plan is superseded by
[Avatar motion direction](2026-11-26-avatar-motion-direction.md).
The current runtime contract is in [Agent avatars](../chat/13-agent-avatars.md).

Status: implemented and verified in this checkout; sole implementation agent,
with direct parent review. All R1–R36 findings are resolved; results are recorded
in the companion evidence ledger. No commits, pushes, merges or shipping actions.
The source reference is the user's
`NyxID bot faces (Copy).html`, supplied on 2026-11-25. The newer reference takes
precedence over this branch's earlier glossy orb design.

## Implementation checklist and evidence

- [x] Read the complete plan and inventory the existing shared model, both
  renderer/editor boundaries, Rust storage/request/view and profile authority.
- [x] Preserve the pre-existing `CLAUDE.md` merge/deployment prohibition diff.
- [x] Verify original fixture: `mockups/nyxid-bot-faces-reference.html`, 56,264
  bytes, SHA256 `9d91953d4e8a97a190ae75ce32f3b9acec1cb94e4024914b8d618a95ef30ea2e`.
- [x] Generate lightweight validation catalogue and geometry/effect data from
  the exact fixture; pin 6/4/6 identity and catalogue coverage.
- [x] Implement portable mask, shared draw list, all appearances and 512px PNG.
- [x] Extend version-aware storage, strict writes and tolerant DTO/client reads.
- [x] Replace orb/Rive rendering with web SVG/Canvas and native SVG stills/quiet
  motion, bounded scheduling and live motion/visibility preferences.
- [x] Complete both editors, analogue explanations, adoption/reset and export.
- [x] Wire honest lifecycle facts, independently scoped voice/transport and
  event-scoped completion/creation reactions across existing consumers.
- [x] Run backend/client validation, render web/native, capture visual evidence,
  update documentation, and resolve direct review findings.

Verification tooling: Node/npm and installed web/mobile dependencies are available.
Rust runs through `PATH=/opt/homebrew/opt/rustup/bin:$PATH`; real MongoDB tests use
the existing replica set and the repository's CI stack size. Independent backend
avatar verification passed 13 tests. Device/client results and all review items
are tracked in the companion implementation evidence document. Temporary logs
are kept under `mobile/review/evidence/` so daemon restarts do not erase them.

## Product contract

NyxBot starts with the reference's round notched head, tall resting eye slots,
and brand-purple `core-nyx` gradient (`#b48cff` → `#5e00f5`) on the
treatment-derived dark background with a tile frame. This is a starting default,
not a restriction: owners can mix every supported head, eyes, palette,
background, style and reaction. The original NyxID logo is an additional
NyxBot-only head. Specialists use the same four notched heads and six eye
choices, with the pinned deterministic default split below. Headwear and
antennae are absent from this kit.

The optional logo preserves `LOGO + NYXBOT_EYES`, eye centers `(166,86)` and
`(220,86)`, radius `21`, and viewBox `-36 -36 330 330`. Do not substitute the
separately framed public brand SVG. Specialist heads retain the original
24×24 space and 60-degree notch. Nondefault logo expressions keep their source
eye design at those anchors with bounded footprints so apertures remain safe.

There are three independent choices:

1. **Identity:** a permitted head, including NyxBot's optional logo, plus saved resting eyes.
2. **Appearance:** palette, foreground style, background and framing.
3. **Reactions:** the visual treatment used for each observed process state.

An expression such as happy or wink can be the saved resting face. A runtime
state such as listening is derived from actual activity. The owner can choose
an animation for listening and preview it in the editor. Saving that choice
does not make the application claim the microphone is listening.

Configuration is per agent and follows existing profile edit authority. The
application's text, approval cards and status controls remain authoritative;
the avatar reinforces those states. Private server activity is not exposed
through avatars to a viewer who cannot already read that activity.

The supplied reference authorizes its extended palette and character-based
treatments for this design. Update the earlier token-only and no-text avatar
guidelines when implementing them; character-based face texture is a narrow
exception, not permission to place arbitrary labels inside the logo.

## Editor

Use the existing agent Avatar editor, with a large live preview and these
groups:

- Face: resting eyes; specialist head choices. NyxBot offers the four heads and its optional original logo.
- Colour: the five named palette sets from the reference.
- Background: solid, gradients, glass, glow, ripple and other supported presets.
- Style: flat, dither or character-based rendering, with variant thumbnails.
- Reactions: Automatic, Quiet or Still presets, plus optional per-state choices.

Show a small curated set first and offer the full catalogue within each group.
Preview chips show Idle, Thinking, Working, Drafting, Listening, Approval,
Success and Failure, with additional connection and creation previews where
applicable. They do not save the current preview state. A small-size preview
shows the actual 16/24/44px fallback alongside the large version.

Randomize previews a new valid configuration until saved. Its face-shuffle
animation belongs in the editor. It does not change a running bot's identity.
Reset restores the default face or the specialist's deterministic face.

## Process mapping

Use a shared resolver over normalized facts. Keep turn, voice, connection and
creation facts distinct; they have different sources and may coexist. For
example, a voice conversation may accept audio while a delegated task runs.
The avatar in the voice surface follows voice activity, while the task keeps
its own status. A connection badge may accompany either without overwriting
the task result.

| Observed fact | Large avatar treatment | Compact equivalent | Source / constraint |
| --- | --- | --- | --- |
| No active process | Blink and gentle glance; optional moving light | Still face or occasional blink | Settled idle state |
| Turn admitted, before text or tool activity | Character churn | Gentle thinking pulse | Existing active-turn state |
| Tool activity running | Character rain or moving dither light | Small working accent | Existing activity statuses |
| Reply text arriving | Character typing | Quiet drafting pulse | Actual streamed text; does not imply audio playback |
| Queued or waiting for capacity | Slow restrained pulse | Static waiting pose | Existing queue/capacity fields |
| Owner action pending | Attentive eyes and a restrained halo | Attentive still | Pending approval/acknowledgement; remains visible if no turn is active |
| Microphone actually accepting audio | Halftone ripple behind a crisp face | Restrained listening ring | Voice capture/input gate; never inferred from an open chat |
| Assistant audio playing | Output-level ripple or pulse | Quiet speaking pulse | Playback state and output level |
| Local connection being established | Scan reveal | Static connecting pose | Scoped transport state, not a fabricated server boot |
| Actual transport reconnecting | Brief channel-offset effect | Static reconnecting pose | Local connection status; no rapid strobing or endless glitch |
| Connection restored | One brief connection ring | Brief static connected accent | Observed reconnect-to-ready transition in that session, not page-wide SSE health |
| Stop requested, not yet settled | Settling motion or static stopping pose | Static stopping pose | Stop request is not proof that execution has ended |
| Successful reply settled while watched | One short hop or happy pose | Brief happy pose | Existing completion identity and settlement tracking |
| Failed reply settled | Concerned eyes or static failure pose | Concerned still | Actual failure; do not treat cancellation as failure |
| Bot creation succeeded in this view | Pixel assembly once | Still face | Creation result; not every mount or fetch |
| Owner tests an appearance | Face shuffle or selected preview | Preview only | Editor event |

Within a text-turn surface, stopping and pending owner action take precedence
over decorative work reactions; pending action must also work without an
active turn. Queue/capacity waiting comes before ordinary work. Actual running
tools take precedence over drafting, then thinking is the active-turn
fallback. Transport uncertainty is shown independently rather than declaring
that a remote task has stopped. Retain the current protection against showing
success before the final reply has arrived and against replaying celebration
on history load or conversation switch.

The current `stressed` mood is based on repeated errors or continuations. Do
not present it as a measured psychological state. Use a neutral retry
treatment rather than an anxious or stressed personality by default. A static
failure expression remains available for an actual terminal failure. Missing
observations do not justify an online, listening, tool-working or success claim.

The current group transcript exposes working member ids and pending actions,
but not each member's full tool/stream/queue state. Keep its reactions to
working/thinking and approval until additional authorized facts are available.
No new polling or inference from elapsed time is needed for the first pass.

## Reference catalogue coverage

Preserve all reference options in the catalogue and implement them in stages.
The first editor need not place every option at its top level.

| Family | Reference options |
| --- | --- |
| Specialist heads | Block, tall, wide, round |
| Resting eyes | Dots, tall, calm, happy, wink, visor |
| Colour treatments | Tint on dark, solid, gradient tile, three-stop, mesh, conic sheen, glass, gradient face, night glow, pastel, neon line |
| Gradient sets | Core, Vivid, Duo, Soft, Deep; seven pairs each |
| Dither | Ordered 1-bit, chunky, fine, error diffusion, halftone, phosphor screen, four-tone handheld, film grain, duotone field |
| Character rendering | Shaded, blocks, Braille, binary, name fill, outline, gradient, hatching, inverted |
| Ripple palettes | Nyx, Dusk, Aurora, Ember, Mint, Ocean, Neon, Midnight, Graphite, Sunrise, Lilac, Seafoam |

Specialists without a saved selection use the reference's hash split:
`tint = hash % 6`, `head = floor(hash / 6) % 4`,
`eyes = floor(hash / 24) % 6`. Pin the ordering and the 6/4/6 constants;
future catalogue additions must not change existing derived identities.
The base reference has 144 specialist combinations before extra appearance
choices. NyxBot starts with its authored purple round face and can select any permitted head; the optional logo adds its original silhouette.

Editor presets describe coordinated foreground/background combinations such
as glass and duotone field. Persist their resolved appearance choices, rather
than a preset whose future definition might change the avatar. The rendering
pipeline is geometry/mask, colour and luminance treatment, surface quantizer,
then compositing over the background. Separating these axes makes a dither
face on a gradient mechanically defined. Catalogue rules handle actual
incompatibilities, such as applying a fill quantizer to a stroke-only outline,
and contrast requirements.

Validate a saved reaction against the states it represents. Each reaction
declares its preferred surface family, vector analogue, quiet analogue and
static reduced-motion frame. Resolve those analogues at render time for the
current style, device and size. Switching from ASCII to a flat face must not
invalidate or erase a saved thinking reaction. The editor explains which
analogue appears with the selected style and at compact sizes.

Palette names carry rendering roles and contrast choices, not arbitrary user
CSS. Do not add destructive red as an identity colour. ASCII and Braille are
decorative pixels; their glyphs are not read aloud as status text.
Preserve the reference's fixed `nyxid·` pattern for Name fill initially; do not
insert arbitrary display names into fixed-width cells.

## Rendering architecture

Retain one avatar boundary and use `face-v1` for the new reference kit. Every agent in the new experience resolves to its saved permitted face, or its current derived default.
The superseded `blob-v1` and unknown templates render a deterministic face
fallback; they do not keep a second visible art system alive. Never write on
read: the stored document remains untouched until an explicit owner save or
reset. An editor that does not understand a template offers explicit adoption
or reset rather than resubmitting a guessed configuration.

Repository inspection found no published branch or pull request for the orb
work. That does not establish the absence of local or other stored records.
No destructive data migration is assumed. During implementation, inventory
consumers and remove the superseded orb/Rive scaffolding where unused, along
with runtime dependencies, build hooks and CSP allowances added only for it.
The new procedural renderer requires no Rive rig or runtime.

The shared implementation owns:

- Original silhouette and eye geometry, default hashing and framing.
- Palette roles, visual presets, labels and compatibility rules.
- Face masks, bounded raster samples, dither thresholds and character grids.
- Pure, deterministic state-to-reaction selection and time sampling.
- Small-size fallbacks, static reduced-motion frames and export composition.

Split lightweight validation metadata (`catalog.json`: ids, agent-kind rules,
field bounds, compatibility) from generated geometry/effect data
(`geometry.json`: paths, masks, timelines). The backend embeds the validation
catalogue; clients share both. Geometry-only changes should not require the
backend to embed all of the artwork. Both files derive from one generator and
are checked for consistency. Emit one platform-neutral draw list for vector
paths, merged pixel rectangles/dots and glyph outlines; web, native and PNG
export consume the same composition.

Web uses inline SVG for flat/vector avatars and Canvas for rich raster effects.
Mobile retains `react-native-svg` and Reanimated for the vector and quiet
animation path. Neither React Native nor the shared library assumes browser
`Path2D`, DOM Canvas, CSS filters, conic gradients or blend modes exist.
Use generated masks or a shared portable sampler, verified against the source
SVG; generate/cache them per geometry, not once per render. Dense character or
pixel sampling does not run through React state on every animation frame.

The initial native rich renderer can use cached static samples or images for
dither and ASCII, plus ordinary eye/whole-avatar movement where compatible.
That is not equivalent to animated character churn, rain or changing halftone
dots. Full native versions are an optional measured renderer spike: first try
bounded precomputed draw-list frames for one focused avatar at no more than
12 fps. Compare a native drawing engine only if that approach fails the
performance budget. If Skia is needed, prove it in both iOS and Android builds.
Avoid embedding the demo HTML in a WebView as the avatar renderer.

Eye cutouts must remain actual transparent holes over every background. The
reference's static even-odd path provides that. For moving eyes, combine the
head and transformed eyes into the fill/mask rather than painting fake holes
with a single background colour. Preserve the visor's pupil islands under the
same parity rule. Each SVG instance needs collision-free mask/gradient ids.
For quiet native blinking/glances, a bounded set of complete even-odd paths
can be pre-mounted and switched with UI-thread opacity. Use discrete switches;
crossfading open and closed paths produces ghosted holes. Do not assume the
current renderer can animate SVG path data on every frame.

Use one bounded scheduler with at most one rich animated avatar per screen,
and stop it when hidden, offscreen, unfocused or backgrounded. Cache masks and stills with explicit
memory bounds. An initial rich-animation target is 12-15 fps; measure on actual
devices before treating that as an accepted budget. Reduce Motion uses a
representative static frame with clear text status; it must not freeze a scan
at a blank or half-assembled face. Changes to motion preference apply live.

Proposed size policy, subject to visual validation:

| Size / surface | Rendering |
| --- | --- |
| 16-20px lists/stacks | Crisp static vector face; no texture or background animation |
| 24-44px focused identity | Vector face with quiet blink/state accent; lists remain static |
| 64-95px cards | Selected static dither where legible; vector fallback for dense character styles |
| 96px+ detail/editor | Full selected style and permitted process reaction |
| 512px export | Complete static chosen appearance, with safe circular-crop margins |

Changing size changes the effective renderer, not the saved preference.
The existing 44px chat header is in the quiet tier on both clients. Rich churn,
rain and typing appear in large details/editor/voice views. The editor
demonstrates this explicitly. Validate exact eye readability and
logo framing at each supported size rather than enlarging the mark casually.

## Expanded voice ripple requirement

Expanded web and native voice views show the selected colourway as a prominent
3:1 halftone field behind a crisp custom face and real status/name. All twelve
colourways, including Sunrise/Lilac/Seafoam light fields, are inspectable in the
editor independently of the avatar's static background. Actual capture/output
observations govern motion. Still, Reduce Motion, hidden/unfocused and background
states retain a complete static field. Native switches a bounded set of shared
precomputed frames on the UI thread, at no more than 12 fps, with one focused
banner; general rich native character/dither animation remains optional.

## Persistence and rollout

Extend the existing human-only agent PATCH, editor access rules, audit behavior
and DTO projection path. Save compact catalogue ids rather than SVG, arbitrary
HTML, animation code, generated images or a transient process state.

Use a bounded generic choices map with strict catalogue-driven writes and
tolerant, lossless projection of well-formed future keys. For example:

```json
{
  "template": "face-v1",
  "choices": {
    "eyes": "dots",
    "palette": "core-nyx",
    "treatment": "gradient-face",
    "surface": "ascii-shaded",
    "background": "gradient",
    "motion": "automatic",
    "react_thinking": "character-churn",
    "react_working": "rain",
    "react_listening": "halftone-ripple"
  }
}
```

These are current catalogue ids. The server-known agent kind governs
eligibility: NyxBot and specialists can select every permitted head; the
original logo remains NyxBot-only. Resting and transient eyes must belong to
the allowed set for that geometry. Defaults fill omitted optional settings
consistently on both clients. Disabled reactions retain the real text status.

Bound the map to at most 32 entries and values to 64 characters, with simple
ASCII identifier keys and values, excluding dots and `$` in keys. Requests
reject unknown keys, invalid ids and disallowed state reactions according to
the current catalogue. Reads preserve well-formed unknown ids/keys for
projection instead of truncating newer choices; malformed stored entries are
handled defensively and never interpreted as code. Store the map as a field,
not untrusted MongoDB update paths.

The original orb model, request, view and client schemas have been replaced
together; face-v1 choices round-trip through every boundary.

Use explicit version-aware read/write handling. Unknown future templates
render a stable fallback and must not be rewritten by an old editor. Enable
authoring only after server readers, writers and DTOs support `face-v1`;
old servers cannot preserve/project new settings just because stored deserialization
tolerates unknown fields. Keep feature rollout separate from configuration
versioning. Existing `assistant:avatars` authority and flag behavior remain
the starting point. Servers that lack face-v1 support cannot safely author or project its choices;
the feature must only be enabled with compatible server readers/writers.

## External avatar export

Provide a deterministic 512px PNG export of the saved appearance at a deliberate
resting frame. Use the same geometry, palette and sampling rules as the live
avatar. Prefer a build-time vector glyph table for the finite character ramps
and fixed Name fill pattern, with Braille dots and blocks drawn as geometry.
This avoids system-font differences, missing glyph ranges and font-load races.
Rasterize the portable shared draw list with the same mask, colour and
compositing rules on both platforms. The native path yields between bounded
raster and PNG-encoding batches while retaining identical output bytes. Snap grid-cell edges
to integer pixels (`floor(i * 512 / N)`) to avoid seams. Export idempotence
includes appearance version, crop, palette and static frame. Offer the chosen
background or transparency with safe crop margins.

Treat upload to Telegram, Lark and Discord as separate platform integrations.
The HTML's note that profile photos are exported when bots are created does
not establish that the existing code or each platform API can set them.
Downloadable PNGs are feasible without those integrations. Profile photos
remain static and are not updated on every thinking/listening transition.
Any managed synchronization needs platform capability checks, existing
credential authority, rate limits and retries. It is outside this design-only
work and must not be described as already supported.

## Implementation sequence and acceptance

1. **Reference fidelity and lifecycle contract.** Capture the supplied HTML
   as a design fixture. Create `face-v1` geometry, palettes and original hash
   defaults in the shared package. Compare exact logo, cutouts, all head/eye combinations
   and small sizes against the reference. Implement and fixture-test the
   process resolver, including no-turn approvals, cancellation, delayed reply
   settlement and voice/transport scope.
2. **Persistence, flat rendering and editor.** Add version-aware Rust
   storage/request/view handling and web/mobile schemas. Render exact face-kit
   vector faces and core backgrounds through the existing boundary. Build
   the preview groups, per-state compatible choices and version fallback.
3. **Static style catalogue and export.** Add all dither/character variants,
   all colour treatments and ripple colourways with cached native stills.
   Resolve native mesh/conic/glow parity through shared sampling or cached
   assets. Verify contrast, masks, glyph coverage, framing and deterministic
   512px export.
4. **Automatic process emotes.** Wire the real text-turn and voice facts,
   visibility-aware rich web renderer and quiet native equivalents. Make
   one-shot effects event-scoped. Document the native rich styles' static
   presentation and the quiet treatment used in the 44px chat header.
5. **Integration completion.** Audit all avatar consumers (chat, groups,
   details, lists, notifications and machine activity), and separately assess
   channel-profile upload support. Full native rich motion is an optional
   measured spike after static and quiet behavior is complete.

Meaningful verification includes shared web/Hermes hash and state fixtures;
reference snapshots at 16/20/24/32/44/112px and 512px; transparent-eye tests over
moving backgrounds and eye survival at every reference raster grid; saved-config
round trips and authorization; unknown
template fallback; no false success/listening/online; dynamic Reduce Motion;
no work offscreen/backgrounded; bounded memory/frame cost on iOS and Android;
and export parity with the chosen static preview. A gallery proves appearance,
but actual lifecycle transitions must also be exercised in chat and voice.
Eye topology tests must match each design: round eyes have two separate holes,
while the visor intentionally has one connected aperture with pupil islands.
Do not impose an incorrect two-hole assertion on every eye variant. Compare
shared draw-list output on web and Hermes and prove the actual PNG export
against the static preview.

## Likely file boundaries

| Area | Existing boundary / proposed extension |
| --- | --- |
| Shared catalogue and geometry | `shared/agent-avatars/`, add versioned face-kit catalogue, mask/sampler and style registry |
| Shared lifecycle and editor rules | `shared/agent-avatars/model.ts`, `mood-fixture.json`, `access.ts`; extract face/activity modules as needed |
| Backend storage and validation | `backend/src/models/assistant_avatar.rs`, `backend/src/services/assistant_avatar_service.rs` |
| Backend request and response | `backend/src/handlers/assistant_team.rs` plus every existing `AvatarView` consumer |
| Web renderer boundary | `frontend/src/components/assistant/avatar/avatar-renderer.tsx` and `nyxbot-agent-avatar.tsx` |
| Web editor and schemas | `agent-avatar-editor.tsx`, `frontend/src/schemas/agent-avatar.ts` |
| Web lifecycle adapters | `frontend/src/lib/assistant/use-agent-mood.ts`, voice hooks/panels |
| Native renderer | `mobile/src/components/avatar/AvatarArt.tsx`, `AvatarSvg.tsx`, `VoiceRippleBanner.tsx`, `useAvatarAnimationActive.ts` |
| Native editor and schemas | `mobile/src/features/agents/avatarEditorModel.ts`, `mobile/src/lib/avatars/avatarSchema.ts` |
| Native voice adapter | `mobile/src/features/assistant/voice/voiceAuraState.ts` and voice conversation surface |
| Product documentation | Update `docs/chat/13-agent-avatars.md` and the avatar section of `DESIGN.md` when implementation replaces the earlier art contract |
