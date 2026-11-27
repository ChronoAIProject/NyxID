# Nyx Natural emotes

`scene.rml` and `rive.yaml` are the editable source for the script-free
`nyx-emotes.riv`. It has 12 ordinary timelines and a state machine named
`Emotes` on artboard `NyxEmotes`. The `AvatarEmote` View Model exposes one
numeric property, `emote`. Stable values and user-facing descriptions live in
`../emotes.ts`.

Rive controls `EyePose` (`x`, `y`, `scaleY`) and 21 hierarchical prop nodes
(position, rotation and scale). The app paints the selected shared vector eye
apertures and a separate SVG prop layer with those transforms. `prop-art.json`
is the single vector-art source: the generator compiles the same shapes into
the editable Rive scene. Prop timing is authored in Rive timelines, with no CSS
animation or application timer for key presses, taps, wrench turns or clock hands.
The art uses simple rectangles, ellipses and polygons drawn locally; no paid
cloud illustration or Rive Editor AI generation was used.
Rive does not replace the configured head, resting eye design, palette,
background, framing, texture, or process resolver. This avoids one rig per
appearance combination and keeps the head and grid fixed. Edge-defined outline and inverted ASCII styles retain still eyes while props
can animate independently. Props do not become part of saved/exported identity.

Typing has a keyboard with coordinated key/tap bursts. Using tools turns a
wrench and bolt together, then disengages/repositions the wrench. Waiting has a
clock hand (decorative waiting, not a countdown). Thinking uses a thought bubble;
approval presents an unchecked request card; retry, stopping and failure have
brief arrow/stop/warning gestures. Reply completion retracts the keyboard.
Listening/Speaking retain the existing twelve voice ripple colourways.

Props now take their material from the selected avatar palette instead of a
fixed white/lavender icon treatment. Full-size textured avatars carry a light
matching grid/grain through the prop fill; compact props keep solid material
for legibility. Marks and clock hands use a contrasting palette ink. The
lower keyboard and smaller clock/cards leave more room around the face.
Typing uses authored uneven bursts, short key travel, small fingertip squash
and longer rests. Wrench strokes have different holds/turn lengths but keep
the bolt engaged until release. The clock ticks and settles; thought dots
breathe subtly rather than bouncing. Eye glances look toward the active prop.


The current Natural web display composites the face, moving apertures and
props into one small Canvas2D image, then quantizes it to at most eight palette
colours with hard alpha. Nearest-neighbour scaling gives every element the
same pixel edges. The 192/112/56px review sizes use 64/56/56 grid cells,
respectively. Positions snap to that grid; no smooth prop layer is visible over
a pixel face. The shared SVG remains the complete load/render-failure fallback.
Rive still authors motion; its poses paint directly without React frame state.
Parsed static paths and colour lookups are cached to avoid repeated glyph parsing.

The comparison page uses this one Pixel art style for all twelve scenes and
one tile framing rule. It randomises shape/eyes/colour without mixing Flat,
Dither and ASCII across the cards. Existing stored appearance preferences
remain the source of identity and material on actual app surfaces.

Full props are used at 96px and above; 44–95px omit small tap details. Below
44px, icons keep the existing face/badge treatment. Quiet mode holds props
still; reduced motion and asset/runtime failure use representative static
props. Still disables the activity props. Home cards remain static.

## Build locally

Use Node 22 or later for the authoring script.
The ordinary frontend build accepts the checked-in asset and needs no Rive CLI.

**Merge gate:** the checked-in `nyx-emotes.riv` is a preview-stream capture,
and its manifest has no authenticated export provenance. A supported runtime
export has not been executed. Source/asset freshness checks establish integrity
only; they do not certify the export path. The draft PR remains blocked until
the author replaces the asset through the authenticated command below, or the
owner removes the Rive runtime from this scope.

```sh
# Sign in with the authoring account and select its workspace first.
# Local runtime export uses one export from that workspace's allowance.
rive login
# From frontend/, with Rive CLI 1.11.0 installed at the selected path:
node ../shared/agent-avatars/rive/generate-props.mjs
npm run build:avatar-emotes -- --cli=/absolute/path/to/rive --workspace=expected-workspace-id
npm run check:avatar-emotes
```

The script verifies and inspects the source, then invokes the documented
`--publish=local --format=json` runtime export from the project directory.
This requires an authenticated authoring account and consumes one export from
the selected workspace. Free workspaces include Rive branding; the selected
workspace must support unbranded exports when that is the required appearance.
The output stays local: the script never uploads the project or hosts a page.
The script rejects any publish warning, missing workspace metadata, and a
workspace that differs from the optional `--workspace` expectation. It never
selects a workspace. The result does not attest the workspace's subscription
or watermark status; the author verifies the required branding in the exported
asset. The compiled artwork is script-free. It loads the result with the pinned web
runtime, samples all 12 emotes, and writes `manifest.json` with compiler,
source/asset SHA256 and byte count. The RML, YAML, prop artwork and generator all participate in the source hash.
The build also checks that the generated RML matches its artwork/timeline source.
The production build checks those hashes before TypeScript/bundling. Rebuild
the asset whenever either source file changes.

Preview transport is not an export API; the authoring script uses only the
documented runtime export. CI checks source and asset hashes without signing
in or consuming exports. An authenticated rebuild records its export method
and workspace id/name/personal metadata alongside the compiler version and
content hashes. Runtime compatibility is pinned to
`@rive-app/canvas-advanced` 2.55.1.

## Integration and checks

Web uses one lazy module worker for Rive/WASM, bounded to 24 scene instances.
Only changed poses are sent, including coordinated parent/child prop transforms;
the DOM updater changes attributes directly without React frame-state updates.
Hidden instances pause while retaining progress; settled timelines stop and
unchanged poses generate no DOM work. Pending state/visibility updates survive
asset loading, and removed pending scenes never start. The last unmount releases
the worker after 1.5 seconds. Still, reduced motion, small icons and runtime failures retain complete shared
vector artwork. Props and eyes preserve their independent material/geometry.

Only the hashed worker asset has `wasm-unsafe-eval` in its response CSP. The SPA
policy remains unchanged. After a production build, run:

```sh
node scripts/verify-rive-worker.mjs
python3 scripts/test-nginx-policy.py
npx playwright test e2e/avatar-emotes.spec.ts e2e/agent-avatar-editor.spec.ts e2e/agent-activity-states.spec.ts
```

The first check serves the actual built worker, rig and WASM under the generated
policies in Chromium and exercises pending updates, cancellation, pause/resume
and interruption. The browser specs cover every emote, real app save/restore,
live states, static Home cards, sticky details, voice colourways, motion
preferences, themes and narrow widths. Unit tests cover material preservation,
exact resting geometry, fallback and lifecycle behavior.

The local comparison entry is `/avatar-emotes.html`; it is excluded from the
production app entry. The normal app uses the same renderer. Native retains
the portable face renderer and voice ripple implementation; this web asset is
not evidence of a tested native Rive integration.
