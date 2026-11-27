# Avatar motion direction: Home, private chat, and Rive

Status: web implementation completed locally on 2026-11-26, following the
user's request to execute and add an all-emote preview. The sections below
retain the design rationale. The implementation now has 12 authored Rive
emotes, configurable shared artwork, static Home cards, a pinned 112px details
preview and voice colourways. State evidence, permissions, identity and
persistence contracts from the earlier plans remain the basis of implementation.

The rig controls eyes and an independent activity-prop layer in a lazy worker.
The follow-up request approved keyboards, working tools and clocks; temporary
props now extend the earlier eye-only direction. Shared finite vector props
are compiled into Rive, whose timelines coordinate their hierarchy. Outline/inverted ASCII retain still eyes with an
editor explanation. Native Rive SDK integration and device performance remain
a separate proof; the web implementation does not claim that parity. See
`shared/agent-avatars/rive/README.md` for reproducible local asset authoring and
`docs/chat/13-agent-avatars.md` for the resulting product contract.

## Problem and direction

The current implementation makes many backend states into repeating body
gestures. Thinking scales the face by 4%, working by 5.6%, writing sways it,
approval lifts it, and retrying tilts it. Idle also breathes on a regular cycle.
Home enables motion on both its greeting and roster cards. The result is
mechanical repetition and competing movement. Increasing amplitudes made this
more visible without improving the character's performance.

The proposed character is calm, attentive, and responsive. Rest is the default.
Eyes and brief transitions carry expression. Most states hold a readable pose
after entering it. A state may be clear through text and a static badge without
needing a unique moving gesture. Voice has a separate, purposeful ripple field.

Keep the reference silhouette, chosen resting eyes, and purple NyxBot default.
Do not add a mouth, permanent limbs or personality inferred from message
content. The later approved temporary activity props extend this original
restriction: keyboard/tap shapes, wrench/bolt, clock and state cards. A completed response is not evidence that the underlying task
succeeded or that a happy emotional response is appropriate.

## Home: /assistant?mock=1

Home summarizes the viewer's activity across conversations. It must remain
comfortable to scan even with many agent cards.

| Surface / situation | Proposed presentation | Trigger and scope |
| --- | --- | --- |
| NyxBot greeting, no active work | Resting face. Optional single natural blink after 8–14 seconds of focused visibility; long still holds between blinks. No breathing loop or repeated greeting. | Visible, focused Home only; Still/Reduce Motion holds the saved face. |
| Agent cards, idle | Saved face, completely still. | No live activity. |
| Agent cards, any active turn | Saved face, static activity mark and short activity label; small steady accent if needed. No card-by-card body loops. | Existing viewer-scoped aggregate activity. |
| Needs approval | Static attention mark and clear Review action. No repeated hop, nod or colour flashing. | Pending owner action, even with no active turn. |
| Work finishes in the background | Update the preview, timestamp and status. An unread/result affordance may persist; do not replay a success animation. | Persisted result/unread evidence, not a timer. |
| While you were away | Static historical avatars and result previews. | History is not live work. |
| Hover / keyboard focus | Existing card/control focus treatment. Optional one brief eye acknowledgement on the primary greeting only; no tracking every pointer movement. | Actual interaction, never a permanent hover loop. |

The greeting can display the same aggregate label when NyxBot has background
work, but its character remains at rest. There is no need for a second animated
copy of a busy NyxBot already shown on a card. Background work never makes Home
appear to be listening to the microphone.

Existing aggregate precedence can stay: approval, working, drafting, retry,
thinking, waiting, stopping, idle. Home only summarizes conversations the
viewer is allowed to observe; voice and connection are not roster activity.
Missing/unavailable observations are unknown, not proof of being idle/online.

## Private chat: /assistant?c=<conversation>&mock=1

The 56px expanded header is the primary animated identity. The compact 20px
header, sidebar, message history and roster copies remain static. The avatar
tile and header layout never move; any small pose change stays inside the art.

| Product state | Evidence | Proposed character / supporting treatment |
| --- | --- | --- |
| Rest / reading | No live turn or pending action | Saved resting eyes. Infrequent blink on the primary visible avatar; no scaling loop, sway or repeated glance sequence. |
| Message accepted | The app has admitted the user's send | One brief eye engagement, then attentive hold. No response to every keystroke. |
| Thinking | Admitted live turn, no more specific fact | Attentive eyes with one small initial glance, then still. Keep the Thinking label. |
| Working | An observed tool is running | Continue the attentive pose; show Working or available factual tool text. Optional small peripheral progress mark; do not bounce the face on every tool call. |
| Writing | Real assistant reply text has started arriving | Relax toward the resting pose while text streams. Writing label/transcript conveys progress. No horizontal sway. |
| Waiting / retrying | Explicit queue/capacity or existing recovery facts | Calm held pose and accurate label. No anxious shaking or “confused” tilt. |
| Needs approval | Pending owner action | Brief attention transition, then held attentive pose. Static badge and approval controls remain clear. |
| Stopping | Stop request accepted, turn still live | Settle the pose; show Stopping until settlement. Do not signal execution has already stopped. |
| Reply completed | A watched live turn settles to a completed reply | Small eye release/settle lasting about 250–450ms, then rest. No default bounce, confetti, forced smile or green success cue for every answer. |
| Failed / interrupted | Actual settled error, or voice-session connection problem | Neutral held face and actionable error/reconnect UI in the correct surface. Cancellation returns neutrally to rest. |

These are product states, not ten independent animation loops. The animation
rig needs five persistent poses: Rest, Engaged, Needs You, Listening, Speaking.
Add short, interruptible timelines for Blink, Engage, Release and an optional
explicit Greeting. Error/stop/wait can hold Rest or Engaged with factual UI.
Existing user-selected reactions remain stored; the proposed Natural default
uses the calmer presentation. Offer expressive alternatives in the editor
without making every page perform them continuously.

The chat header observes this conversation only. If the same agent works in
another thread, its Home/sidebar aggregate can differ honestly; the label must
make that scope clear. Voice observations override only the active voice
surface, not the text turn resolver.

## Timing and interruption

Initial values for visual review, not settled constants:

- Engage/release transitions: 150–300ms, with a brief 250–450ms settle after a
  watched completion. Avoid rubbery overshoot and perpetual sinusoidal motion.
- Blink: asymmetric close/open over roughly 120–180ms, scheduled irregularly
  with at least several seconds of stillness. Reduced Motion disables it.
- No automatic body scale/rotation/translation loop in Natural mode. If a
  particular shape needs a tiny settling deformation, author it once on entry.
- State labels and controls update immediately. An animation may suppress
  fleeting decorative transitions for 200–300ms, but must never delay approval,
  stop, failure, microphone or playback status.
- New facts interrupt the current transition from its current pose. Do not
  finish a greeting/acknowledgement before showing a pending action.
- Repeated snapshots of the same turn do not replay its entry reaction.
  Completion events are keyed by conversation/turn identity and observed once.
- Navigating to history, refocusing, remounting or reconnecting does not replay
  past success/greeting. An active new state supersedes an old one-shot.
- Pause hidden/offscreen/background surfaces, and snap to a complete current
  pose when restoring them. Unknown or stale facts never generate fake activity.

## Voice: preserve the ripple colourways

| Voice phase | Evidence | Character and field |
| --- | --- | --- |
| Connecting / reconnecting | This voice session's transport phase | Resting character, quiet/static field, honest connection label. |
| Listening | Microphone/input gate is actually accepting audio | Attentive pose; slow outward halftone ripple in the selected colourway. Audio-reactive intensity only if reliable input level is exposed. |
| Waiting for response | Actual voice-turn processing after input, with mic/playback gates inactive | Engaged hold; field settles to low activity. It must be distinguishable from listening and speaking. |
| Speaking | Assistant audio is actually playing | Selected ripple responds to smoothed playback level. Optional slight eye animation; no invented mouth/lip sync. |
| Muted / capture inactive | Input gate inactive, no output playing | Still field/face and explicit muted/inactive status. |
| Ended / failed | Actual lifecycle event | Settle field and return to text identity, with error UI when needed. |

Keep all twelve ripple colourways independent of face colour, including the
light variants. Colourway affects styling, never microphone state. Smooth
level input with a starting attack of 60–100ms and release of 180–300ms to
avoid noisy frame-by-frame pumping; tune against real playback. Under Still
and Reduce Motion use one complete field plus explicit state text. A Rive
prototype can retain the existing ripple renderer; re-authoring the field in
Rive is a separate comparison, not a prerequisite for better character motion.

## Where Rive helps

Current official documentation supports an AI Agent inside the Editor that
imports SVG, rigs artwork, creates timelines/state machines and binds data.
The CLI also creates local .riv assets from editable RML source. Rive therefore
can provide authored easing, pose blending, transition interruption, eye
animation and reusable components. It will not choose appropriate product
states or make every repeating gesture feel natural automatically.

Recommended prototype:

1. Import the exact round-head/tall-eye reference geometry, retaining the
   notch and genuine even-odd eye holes. Build Rest, Engaged and Needs You
   first, plus Blink/Engage/Release timelines. Work at the real 56px and 72px
   display sizes, with a larger inspection view beside them.
2. Use a script-free state machine and View Model data binding. Keep RML and
   generated .riv under version control with a small asset manifest and build
   instructions. The Editor AI Agent is also a valid authoring route; its
   output gets the same visual and structure review.
3. Bind an application-owned `pose` enum, `surface` enum (home/chat/voice/editor),
   `motionAllowed` boolean, chosen appearance properties, and `audioLevel`
   number. Fire one-shot triggers only from observed event identities.
   Keep authentication, turn precedence, approval, stop and lifecycle logic
   in the application. Animation events never authorize or complete work.
4. Use separate rig layers for eyes/expression, an optional internal settle,
   and a peripheral accent. No blanket blending of complete images: that
   ghosts the eye holes. Corresponding paths can interpolate; incompatible
   head/eye variants switch at rest. Test intermediate eye geometry.
5. Compare the authored rig with the calmer existing renderer in the actual
   Home and chat layouts. Rive owns its display-frame progression; do not
   sample its animation through the existing 12fps React scheduler. Only send
   state/appearance changes and throttled smoothed audio levels to the rig.

The current docs recommend @rive-app/react-webgl2 for most React integrations.
Select and pin the runtime after proving the required features, teardown,
loading cost and local WASM/asset delivery. Bind appearance before revealing
the first Rive frame, so the default art cannot flash over a saved face.
Preserve existing static rendering during load, failure and unsupported cases.

At the time of the original proposal there was no .riv file or Rive dependency
in the checkout. `rive` was not
on PATH and is absent at the documented ~/.rive/bin/rive location. Installing
the authoring tool and building the local prototype are implementation work.
Script-free local builds avoid the script-signing/publishing path; this plan
requires no Rive cloud publishing, deployment or release.

## Configurable shapes, eyes, colours, Dither and ASCII

The user-selected appearance remains the identity. A Rive integration must not
replace a configured avatar with one fixed mascot or restore the old orb.

| Choice | Proposed Rive treatment |
| --- | --- |
| Head | Finite authored variants for block/tall/wide/round and NyxBot's optional Nyx head, each with its own eye anchors. Reuse components and one binding contract. |
| Resting eyes / emotes | Selected eye design is the baseline. Animate compatible eye geometry; temporary pose returns to those chosen eyes. No permanent tall-eye override. |
| Palette / finish | Bind palette roles and finite finish variants; purple is only NyxBot's default. |
| Background / frame | Independent host or rig layer; does not squash or shift the outer tile. |
| Dither / ASCII | Preserve the existing shared texture output. First prototype a cached alpha image in the rig or an aligned host layer, with expression/peripheral motion where compatible. |
| Ripple colourway | Preserve the independent selected field and its twelve palettes. |

Rive supports dynamic image data binding, but runtime-referenced SVG is not
supported. Exact vector heads must be authored into the asset; existing texture
compositions can be rasterized once per appearance change. A flattened texture
image cannot produce independently moving eye holes by itself. Before promising
textured eye animation, prove a head-colour texture separated from the live eye
mask, with matching alignment, antialiasing and genuine transparency. If that
proof fails, keep the textured face still and animate only the surrounding
state/voice accent. The editor explains that treatment; no saved choice is
erased or silently displayed as Flat.

ASCII/dither pixel grids should not bend and stretch as though made of rubber.
Their own optional character-churn/halftone reactions remain deliberate rich
style choices, previewed separately from Natural motion. Do not create an
asset for every head × eye × palette × texture combination.

## Implementation sequence and review gates

1. **Calmer local baseline.** Remove the default breathing, sway, tilt and
   repeated lift from these surfaces; make Home cards static. Retain truthful
   state labels, static badges, chosen styles and voice ripple colourways.
2. **One real Rive motion study.** Round head/tall eyes in the actual header
   and greeting, including instant interruption, Rest/Engaged/Needs You,
   and single entry/release reactions. Review sustained stillness as carefully
   as moving frames. No dependency added to all avatar instances yet.
3. **Configuration proof.** Verify all five heads, six eye styles, palette
   changes, true holes on light/dark backgrounds, and at least one Dither and
   one ASCII configuration. Resolve the texture/mask route before broadening.
4. **Voice proof.** Bind actual input/playback phases and smoothed levels;
   compare the existing ripple renderer with an optional Rive field using
   all twelve colourways. No synthetic audio or fictional listening phase.
5. **Focused web integration.** One active character scene for the focused
   Home greeting, chat header or expanded voice surface. Other copies use
   shared static artwork/badges. Cache assets, release runtimes on navigation,
   handle load errors and WebGL loss, and respect motion preferences live.
6. **Native proof.** Rive has a React Native runtime; Expo needs a development
   build rather than Expo Go. Validate the selected SDK/features in both iOS
   and Android builds before claiming native parity. Keep portable static and
   voice behaviour available while that proof is pending.

Review gates:

- A 20-second idle recording is predominantly still, with no scale pump,
  sideways sway, repeated approval hop, or synchronized row of moving faces.
- All states remain understandable in Still/Reduce Motion and with the rig
  unavailable; approval and error controls remain immediate and usable.
- Live observed turn transitions, stop during transition, approval between
  turns, retry, cancellation, historical load and account/conversation switch
  preserve the existing evidence and event-deduplication guarantees.
- Identity at rest matches the saved avatar. Eyes retain required topology at
  every intermediate frame; small sizes, light/dark themes, Dither and ASCII
  do not change to an unrelated face.
- Evaluate 56px chat and 72px Home at desktop and 320px/390px widths. Measure
  frame time on a target device (target smooth display-rate playback), dropped
  frames, runtime/asset transfer size and memory; no heavy runtime per roster
  item. Hidden scenes do no frame work. Release the runtime on unmount.
- Voice has no listening animation while capture is inactive, no speaking
  animation without playback, and no persistent fake connection/success badge.
- Rive motion quality must earn its integration through the local comparison.
  Complete the small rig and texture proof before expanding runtime scope.

## Suggested first Rive AI brief

Use the exact imported NyxBot reference vector. Preserve its silhouette, notch,
purple palette and transparent eye holes. Create a calm assistant with five
poses: Rest, Engaged, Needs You, Listening and Speaking. Begin with Rest,
Engaged and Needs You. Rest is still. Add an individually triggered natural
blink, a small eye engagement on message acceptance, and a short release back
to Rest after a reply. Hold poses between events. Use 150–300ms eased,
interruptible transitions. Keep the outer avatar tile fixed. Avoid recurring
body pulses, bouncing, horizontal sway, anxious shaking, added anatomy or
automatic celebration. Expose a pose enum, appearance controls, motionAllowed,
audioLevel and explicit one-shot triggers through a View Model. Author ordinary
timelines and state machines without scripts. Preview at 56px and 72px.

## Verification and official references

Reviewed current shared activity/process/presentation contracts, Home and chat
call sites, and the authored motion signatures. The supplied chat screenshot
anchors the existing header layout. The literal mock conversation ID in the
request was unavailable in the separate inspection browser; that browser's
own Morning briefing fixture provides the same chat surface. Mock IDs are
session-specific. This section records the original design review; the status
and implementation evidence above describe the later working Rive integration.

Official documentation consulted 2026-11-26:

- [Rive AI Agent](https://rive.app/docs/editor/ai-agent/ai-agent)
- [CLI and editable local RML workflow](https://rive.app/docs/cli/overview)
- [CLI setup, local builds and script-signing distinction](https://rive.app/docs/cli/getting-started)
- [React runtime](https://rive.app/docs/runtimes/react/react)
- [React View Models, enums, images and first-frame binding](https://rive.app/docs/runtimes/react/data-binding)
- [Transition easing and interruption](https://rive.app/docs/editor/state-machine/transitions)
- [Even-odd fill rules](https://rive.app/docs/editor/fundamentals/fill-and-stroke)
- [Runtime asset loading and SVG limitation](https://rive.app/docs/runtimes/react/loading-assets)
- [Feature support by runtime](https://rive.app/docs/feature-support)
- [Expo development build requirement](https://rive.app/docs/runtimes/react-native/adding-rive-to-expo)
