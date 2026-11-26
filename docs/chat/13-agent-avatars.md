# Agent faces and voice colourways

`assistant:avatars` enables the shared `face-v1` kit on web and mobile. NyxBot
starts with the reference's round notched head, tall eyes, brand-purple
`core-nyx` gradient face, treatment-derived dark background, and tile frame.
This is a starting appearance: owners can mix heads, resting eyes, palettes,
backgrounds, framing, rendering styles, and per-state reactions. Randomise
includes every head permitted for the agent, including the original NyxID logo
for NyxBot. Specialists retain the pinned 6/4/6 identity split described below.

The source is the byte-preserved
[`mockups/nyxid-bot-faces-reference.html`](../../mockups/nyxid-bot-faces-reference.html),
SHA256 `9d91953d4e8a97a190ae75ce32f3b9acec1cb94e4024914b8d618a95ef30ea2e`.
The kit replaces the superseded orb art. Web Natural motion uses a locally
authored, script-free Rive state machine to control the shared vector eye
apertures; materials remain in the portable painter. Native retains its
portable renderer. Neither client needs an external font or avatar WebView. While the flag is off, the existing
brand mark and specialist initials remain the rollout fallback; saved choices
are retained.

## Web editor

The actual `/assistant` agent-details Overview shows a compact saved-avatar
tile. Edit avatar opens a separate page inside the drawer, with three guided
steps: Face (head and eyes), Style (palette, texture and background), and
Motion (motion mode, expression preview and all twelve voice ripple colourways).
Back to Overview checks for unsaved avatar changes before leaving and restores
focus to the tile. Unrelated profile drafts remain intact.
Back/Next and the numbered step buttons revisit the same unsaved draft.
Step navigation, browsing a palette family and previewing expressions never
mark the form dirty or write preferences.

Palette families show one row of swatches at a time. Flat, Dither and ASCII
are the texture families; the selected family exposes its variants. Finish
and frame live under More appearance options, per-state reactions under
Custom reactions. Collapsed choices stay in the draft and save payload.
Each step has a numbered progress indicator, a title, a short instruction
and a named next step. The details header starts with a 112px avatar and
collapses to a 32px avatar/name row when the active page scrolls. The status
line remains aligned under the name; scrolling to the top restores the large
avatar. The header previews the draft while the editor is open.
The standalone editor retains a 96px preview. The former In the app preview
group is removed. Saved textures remain visible in small icons. The
NyxID-derived head is labelled Nyx; the stored `logo` choice id stays unchanged. The Motion step also previews the voice ripple
field. Downloads are not part of the web editor.

Every step shares one sticky action row at the top: Save avatar, plus Discard
while there is a draft. Save reviews only the appearance; Discard restores the
last confirmed appearance. Native form submission also opens the review.
The drawer-wide save bar is hidden while the avatar page is active, and
returns with any unrelated drafts on Back. Dismissal guards still protect the
avatar draft and pending writes. Save status and errors stay with the actions.
Saving reports progress, failures and completion in the editor. Confirmed
writes update cached roster, details and crew appearances immediately and
cancel older reads before they can restore the previous choice. The chat
header and voice panel take the current roster appearance, including null
resets, ahead of a conversation's older snapshot. Reset restores NyxBot's
round head, tall eyes and brand purple.

## Data, authority, and compatibility

The human-only agent PATCH keeps existing profile authority: maintainers and
shared managers may edit an active agent; viewers, shared users, destroyed
agents, and conversation/API keys cannot. The acting person's rollout flag
also gates writes. Object sets, null clears, and omission keeps the avatar.
Audits retain metadata only. Lists, conversation identities, groups,
notifications, voice discovery, and machine activity project the same view
without additional reads.

The stored, request, and response shape is:

```json
{
  "template": "face-v1",
  "choices": {
    "head": "round",
    "eyes": "tall",
    "palette": "core-nyx",
    "treatment": "gradient-face",
    "background": "treatment",
    "surface": "flat",
    "frame": "tile",
    "motion": "automatic",
    "ripple_palette": "nyx"
  }
}
```

`shared/agent-avatars/catalog.json` contains validation metadata: ids, labels,
kind rules, bounds, compatibility, defaults, and reaction analogues. The Rust
service embeds this lightweight file. Writes reject unknown templates, keys,
ids, incompatible outline textures, and entries whose `kinds` exclude the
server-known agent kind. The map has at most 32 entries; identifiers have at
most 64 ASCII characters, begin with a lowercase letter, and use lowercase
letters, digits, underscores, and hyphens. Dots and dollar signs are rejected.
The map is one MongoDB field, never a set of untrusted update paths.

Reads preserve bounded, well-formed future keys and values. Malformed stored
entries are ignored defensively. Unknown templates, including legacy `blob-v1`,
resolve to a stable current default without rewriting storage. An editor must
explicitly adopt or reset an unknown appearance before it can save. Shared
validation fixtures exercise equivalent Rust and TypeScript decisions.

Editors capture the raw stored envelope for semantic conflict checks. Map
ordering does not create a conflict. They lock the complete preflight/write,
retain the last successful save for export/discard even when parent props lag,
and fence identity, account, and stale asynchronous completions. Web writes use
an owner-bound mutation and HTTP authority guard; native writes retain the auth
epoch through the request guard.

## Reference catalogue

- Heads: block, tall, wide, round; the original NyxID logo is an additional
  NyxBot option. Original specialist paths use their 24×24 space and 60° notch.
- Eyes: dots, tall, calm, happy, wink, visor. Eyes are actual even-odd holes;
  visor pupils are solid islands within one connected aperture.
- Colour treatments: tint, solid, gradient tile, three-stop, mesh, conic sheen,
  glass, gradient face, night glow, pastel, neon line.
- Palettes: Core, Vivid, Duo, Soft, Deep, seven pairs each. No destructive-red
  identity palette is introduced. This approved extension is specific to avatars.
- Dither: ordered 1-bit, chunky, fine, error diffusion, halftone, phosphor,
  four-tone handheld, film grain, duotone field.
- Characters: shaded, blocks, Braille, binary, name fill, outline, gradient,
  hatching, inverted. Finite vector glyphs and geometric blocks/dots are
  decorative pixels and are never read aloud. Name fill stays `nyxid·`.
- Voice ripple: Nyx, Dusk, Aurora, Ember, Mint, Ocean, Neon, Midnight, Graphite,
  Sunrise, Lilac, Seafoam. The last three have light fields and dark text.

`background: treatment` follows the selected treatment's reference background.
An explicit background remains independent when the treatment changes. Genuine
outline/texture incompatibilities have accessible explanations; no other choice
is silently erased. An explicit mix that would hide the face resolves to a
contrasting existing palette ink at draw time. Transparent compositions use the
ambient theme or voice disc for that decision. Reference-derived treatments
retain their authored colours.

Specialist defaults use FNV-1a over UTF-16 code units:
`tint = hash % 6`, `head = floor(hash / 6) % 4`,
`eyes = floor(hash / 24) % 6`. The original orders and divisors remain pinned
when the catalogue expands. The six source tint roles retain tint on dark and
solid on light. Saved configurations are resolved by the server-known kind;
NyxBot's saved head is honoured.

## Composition and size policy

Run `node shared/agent-avatars/generate-face-kit.mjs --check` to verify the
catalogue and geometry against the original fixture. The generator supports the
frontend's documented Node 20 minimum. `geometry.json` carries paths, bounded
contours, palette roles, and effect constants; the backend does not embed it.
Original SVG strings are preserved. Generated contours round to five decimals,
below 0.00022 export pixels in the 24-unit space. Optional logo emote eyes stay
at the original anchors; nondefault eye footprints retain a safe bridge.

`draw.ts` emits one portable 512-unit composition. It combines exact vector
masks, colour/luminance treatment, grid quantizers, vector glyph outlines, dots,
and sampled background/foreground pixel planes. Web and native consume that composition;
PNG export rasterizes it directly. The painter order is background plane/paths,
masked foreground plane, then foreground details and accents. Pixel planes avoid seams between independently
antialiased gradient cells at arbitrary display scales. Silhouettes and eyes
retain their own vector antialiasing and transparent cutouts.

| Size | Effective rendering |
| --- | --- |
| 16–20 | Selected appearance, static |
| 24–95 | Selected appearance with permitted quiet expression/accent |
| 96+ | Selected appearance, with permitted reactions |
| 512 export | Complete saved appearance at its deliberate resting frame |

Coarse texture grids are checked for each head/eye design. A grid that loses
aperture topology uses a crisp vector analogue and the editor explains why;
the selected preference remains stored. Tests cover every reference raster
grid, including the visor's distinct topology. Native rich textures remain
static; ordinary eye and whole-face movement is applied only where compatible.
Open, closed, left-glance and right-glance vector frames switch discretely on
the UI thread, without crossfading holes.

Mask caches have a fixed entry/byte limit; still caches are bounded by both
entries and bytes. One web scheduler admits at most one rich animation per
screen, gives the voice banner priority, and stops completed one-shots and
hidden/offscreen work. Native quiet work stops when unfocused/backgrounded and
uses live Reduce Motion. Natural web reactions reuse cached texture geometry and pixel planes; only
vector eye masks change. Explicit custom reactions retain the bounded rich
painter. Textures do not flow through React state every frame.

Web home uses a 72px greeting avatar and 56px roster avatars with more padding
and space around them. The expanded chat identity is 56px; its compact scrolled
header stays 20px and static. Home cards are still, with factual activity
badges. The greeting uses the resting face with occasional blinks. Natural
motion has no repeating body scale, sway, tilt or hop. Still and reduced
motion retain the complete saved resting appearance.

## Honest process reactions

`activity.ts` resolves text-turn facts in the order shown below. The shared
fixture verifies the TypeScript and Rust projection. The badge supplements the
existing status text and approval controls. Assistive technology gets state
phrases through the owning controls, with the badge itself decorative.

| State | Observed fact | Badge | Web Natural presentation |
| --- | --- | --- | --- |
| Stopping | A live turn has a stop request | Square | Settle into a held pose |
| Waiting for approval | The conversation has a pending acknowledgement, including between turns | Exclamation | Open, attentive eye hold |
| Waiting | The turn is queued or waiting for capacity | Hourglass | Calm lowered eye hold |
| Working | An observed tool activity is running | Bolt | Focused eye hold |
| Writing | The turn contains reply text or the open transcript has streamed text | Lines | Eyes follow the reply |
| Retrying | The turn has a continuation or at least two tool errors | Circular arrow | Brief glance, then a hold |
| Thinking | A live turn has none of the earlier facts | Three dots | Upward glance, then a hold |
| Idle | No earlier fact applies | None | Resting face |

A newly observed reply completion briefly releases the eyes toward rest. It
adds no default success check or celebration; an explicitly saved success
reaction can still request one. A settled failure can show a cross. A cancelled turn settles neutrally. History
loads, conversation switches and refocus do not replay completed one-shots.
The transcript can retain an honest stopping or failed state for a destroyed
agent.

### Roster aggregation and visibility

The human roster includes optional per-thread `activity`. The web activity
read at `GET /api/v1/assistant/nyxagent/agents/activity` provides the same
viewer-scoped facts without reloading the roster. Each entry contains only a
conversation ID and state, grouped under the agent ID. Reads apply existing
personal, organization-use and live-share authority. Approval-only entries
leave the live-turn status and model-facing tools on their original contract.

Across an agent's conversations, approval outranks working, writing, retrying,
thinking, waiting and stopping, in that order. Destroyed
agents resolve to idle in the roster. On web the open transcript replaces only
its own conversation's entry for the same agent. The newest complete snapshot
provides the remaining entries. Mobile uses the roster projection. Older
servers fall back to the coarse pending count and running flag when activity
is absent.

Background proxy approvals matched by chat-key name are visible as approval
in the open transcript. In the background their avatar follows the observable
live tool state. Conversation, group and notification events refresh web
activity on a coalesced one-second cadence. A card expiry or change without one
of those events can remain stale until the next existing refresh, with no fixed
wall-clock bound. Group transcripts expose working member IDs and pending
actions. The member decoration uses only those coarse facts. Voice stays
separate and uses actual capture or playback observations and its own local
transport state.

### Renderer contract

Web Natural motion uses `@rive-app/canvas-advanced` 2.55.1 in one lazy module
worker, with a local script-free 47KB `.riv` asset. The 12 emotes are Resting,
Thinking, Typing, Using tools, Waiting, Needs approval, Retrying, Stopping,
Error, Listening, Speaking and Reply complete. The existing observed-state
resolver selects a stable numeric binding; the Rive state machine authors
interruptible easing, eye movement and holds. It never infers product state.
The tile, silhouette, paint and texture remain fixed. A separate prop layer
now renders a keyboard with coordinated taps/key presses, a working wrench and
bolt, a waiting clock, thought bubble, approval card, retry arrow, stop tile and
warning tile. Reply completion retracts the keyboard. The shared prop geometry
is compiled into Rive; its hierarchy and all motion come from the rig. The web editor labels
these default reaction mappings Natural while retaining their compatible stored
ids; deliberate expressive alternatives keep their own labels.

A shared vector mask preserves real holes, including visor pupil islands.
A head-material patch fills only the region vacated by a moving aperture;
ordinary material stays byte-for-byte at rest. Outline and inverted character
styles keep static eyes because their material is defined by the aperture's
edge. The editor explains this without changing the saved style; activity props can
still animate independently. Explicit
custom reactions continue through the portable expressive painter.

Still, per-state Still, reduced motion and sizes below 24px do not load Rive.
Props use a simpler treatment at 44–95px and the full scene at 96px+. Quiet
mode holds props still; reduced motion and runtime failure keep a representative
static prop, while Still disables props. Below 44px there are no activity props.
Props are transient and excluded from saved/exported identity.
Prop material follows the avatar palette and effective texture, with contrasting
marks and simpler solid fills at compact sizes. The keyboard is low-profile,
tap shapes are small and soft, and secondary props have thinner edges. Authored
uneven typing bursts, varied wrench strokes, ticking clock holds and glances
toward the active prop replace uniformly repeated gestures.

Hidden, covered, unfocused and offscreen scenes pause without replaying entry
reactions. The worker admits at most 24 instances, stops settled timelines,
and emits only changed poses. It terminates 1.5 seconds after its last scene
unmounts. Load/runtime failure restores a complete vector still and releases
the failed session. Recovery is bounded to two attempts after two and four
seconds, deferred while hidden and cancelled on unmount. Main-thread
React never receives frame state. Native Rive integration is a separate proof;
no native Rive runtime or parity claim is added here.

The SPA CSP retains its existing restrictions. Only the generated Rive worker
asset receives `wasm-unsafe-eval`; scripts and assets remain local. Build-time
source/asset hashes reject a stale rig. See
[`shared/agent-avatars/rive/README.md`](../../shared/agent-avatars/rive/README.md)
for authoring and verification.

Natural web emotes now compose the complete scene into a small Canvas2D image
with at most eight palette colours and hard alpha. The face, moving eyes and
props share that grid. Nearest-neighbour scaling keeps stepped edges at app
and preview sizes; positions snap to the same grid. Static path parsing and
colour classification reuse bounded/weak caches. SVG artwork remains the
complete rendering fallback, with the accessible name exposed only once.

The local comparison page is `/avatar-emotes.html`. It renders all 12 emotes
on randomized configurable avatars in one Pixel art style, with same-avatar
comparison, pause/replay, themes, actual 56px/112px size comparisons and all
twelve voice colourways. The mixed texture selector has been removed. It uses
the actual app renderer and does not access a microphone or save preferences.
It is a development entry and is excluded from the production app bundle.

One-shots identify both the state and the observed event. Pausing preserves
progress; completed events do not replay on refocus or configuration updates.
A fresh editor click can preview the same state again. Scan completion uses its
own full-reveal duration. Successful client-side creation records a short-lived
in-memory event only after the create request succeeds. One focused eligible
avatar may consume it while idle; real activity supersedes it permanently.
Roster/history reads and reloads cannot recreate the event.

## Voice ripple banner

Expanded web and native voice surfaces use a prominent 3:1 halftone banner:
custom vector face on the left, actual status/name on the right. Its saved
`ripple_palette` is independent of body palette, style, and static background.
The editor previews every colourway without changing those other choices.
Compact voice identities keep their quiet treatment.

The shared radial field follows the reference equations. Native precomputes 12
384×128 frames for one palette and switches image opacity on the UI thread at
about 7.6 fps. One focused banner holds the motion lease. Actual playback level
modulates the field without restarting its clock. The web uses the same frames
through the bounded canvas scheduler, with an 80ms attack / 240ms release
on playback-level intensity. Idle, Still, Reduce Motion, hidden,
unfocused, and background states show a complete representative static frame.
Frame preparation happens outside render and yields between frames, retaining
a complete vector poster while preparing. A stale palette preparation is
cancelled. The frame cache retains one palette, with an explicit byte limit. A rich native
drawing engine and general animated character/rain parity remain optional.

Web transport observes reconnecting→ready within the actual voice session and
briefly displays a connection accent even while capture continues. It does not
infer restoration from page-wide SSE health. The native voice hook currently
exposes connection establishment and actual input/output gates, without an
equivalent reconnect/restored event; native does not fabricate those facts.
Existing status controls remain authoritative and provide the accessibility
announcement; decorative banner text and glyphs do not duplicate it.

## Deterministic export and checks

The portable export path and native editor export/share a 512×512 RGBA PNG of the last successfully saved
appearance, with the selected background or transparency and safe crop margins.
The same finite glyph geometry, masks, palette, field samples, framing, theme,
and resting frame are used on both clients. The portable encoder uses
uncompressed DEFLATE and explicit CRCs, with no timestamps, system fonts, or
platform image codecs. PNG output is deterministic for the appearance and export
options. Native export yields between raster rows/path batches and PNG
copy/checksum blocks, shows preparation, guards repeated taps, and fences file
creation/sharing against changed account or unmount. Sync and async encoders
drain the same steps and produce identical bytes. Telegram, Lark, and Discord
profile upload/synchronization remains a
separate integration; export does not claim to update channel profile photos.
The web editor deliberately has no download action.

The implementation/evidence ledger is
[`docs/plans/2026-11-25-nyxbot-face-kit-evidence.md`](../plans/2026-11-25-nyxbot-face-kit-evidence.md).
It distinguishes executable results from outstanding device/visual checks.
The isolated `mobile/review/face-kit-entry.tsx` entry uses the actual native
renderer and Hermes to verify contracts, output hashes, exports and cache cost;
it is not imported by the normal app entry and contacts no backend.
