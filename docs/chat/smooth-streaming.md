# Smooth Text Streaming

Last verified against the phase-2 fix commit (2026-08-07).

The presentation seam now receives the canonical accumulated content string
from `RuntimeEventAccumulator` through `ChatMessage` and `TextBlock`. Smooth
reveal remains strictly above that string: it never parses SSE, reduces actor
facts, owns terminal state, or changes persisted/history content. Direct chat
adapts its accumulated text to the same `ChatMessage`/`TextBlock` surface.

This document is the single specification for how streamed assistant text is
revealed and rendered. Section 2 describes the baseline shipped in PR #1390;
sections 4–5 describe the implemented boundary-safety and adaptive-spread
deltas. The delta implementation is commits `1dee68e6` through `41678074`;
section 6 retains the work-package record and verification contract.

It supersedes and replaces `smooth-streaming-plan.md` (v2) and
`smooth-streaming-plan.review.md`, both deleted. Audit note: that plan was
written under an explicit no-repo-access constraint, before PR #1390 was
recognized as already landed, so much of it analyzed a naive frame-derived
typewriter this codebase never had. Its findings that remain true against the
real baseline are folded into sections 3–5 as design rationale; section 7
records the disposition of the rest so the deletion loses nothing.

## 1. Problem

Upstream text deltas arrive in bursts at whatever cadence the network and the
Aevatar projection scheduler produce. Painted verbatim, an answer advances in
visible steps rather than flowing. The design goal, then and now, is
**decoupling the rate text appears from the rate it arrives** — while never
adding enough latency to make a fast answer feel slow, which is the failure
mode that gets this class of feature reverted.

## 2. Implemented baseline (normative)

Three cooperating pieces, all shipped in PR #1390 with colocated tests.

### 2.1 Pacing controller — `frontend/src/hooks/use-smooth-reveal.ts`

`useSmoothReveal(text, active)` returns a prefix of `text`. While `active`, a
single `requestAnimationFrame` loop advances a revealed-length state against
the frame clock; the frame loop reads the newest arrival through a ref so it
is never torn down per delta.

Per frame, with `backlog = arrived − revealed`:

```
if backlog > SNAP_BACKLOG_CHARS:  revealed = arrived        // snap
rate    = max(MIN_CHARS_PER_SECOND, backlog / (DRAIN_MS/1000))
advance = max(1, round(rate × min(frameSeconds, MAX_FRAME_SECONDS)))
```

| Constant | Value | Why |
| --- | --- | --- |
| `DRAIN_MS` | 90 | Backlog drains over ~this long, so display lag is small and constant; a fixed lag reads as flow, a chunk-size-dependent lag reads as stutter |
| `MIN_CHARS_PER_SECOND` | 700 | Floor so the last characters of a chunk still visibly move |
| `SNAP_BACKLOG_CHARS` | 1500 | A jump this large is a load (re-mount, reconnect, history projection), not a stream; typing it out would lie about when it arrived |
| `MAX_FRAME_SECONDS` | 0.05 | A longer frame is a stall; crediting it in full would jerk the text |

The controller is **time-derived, not frame-derived** — it computes characters
owed from elapsed wall-clock, so display refresh rate does not set the reveal
rate. (This is the defect in the Upstash reference implementation; the
baseline never had it.)

Guarantees, each held by `use-smooth-reveal.test.tsx`:

- **Settle hand-over.** The moment `active` goes false — turn settled,
  cancelled, or errored — the full text is returned synchronously. Nothing is
  ever stranded behind the animation, and a cancelled turn stops typing
  immediately.
- **Reduced motion.** `prefers-reduced-motion: reduce` (or no
  `requestAnimationFrame`) disables pacing entirely.
- **Prefix-only output.** The returned string is always a prefix of the
  arrived text.
- **Replacement clamp.** A block whose text was replaced shorter never slices
  past its own end.
- **Resume without re-typing.** Crossing back into paced mode (an approval
  continuation appending to a settled block) starts from what is already on
  screen and streams only the new suffix.
- **Log-time convergence.** Drain rate is proportional to backlog, so backlog
  decays exponentially; six times the text costs well under twice the frames.

### 2.2 Stable-prefix markdown split — `frontend/src/lib/assistant/markdown-stream.ts`

Re-parsing the entire accumulated markdown per paint is linear in length, so
long answers write themselves progressively more slowly. `splitStableMarkdown`
splits the revealed text into a settled `prefix` (parsed once, memoized by
string identity) and a live `tail` (re-parsed per frame).

It is deliberately conservative — markdown is not a prefix-stable language
(setext headings, link-reference definitions, tables, list tightness and
fences all let later text reinterpret earlier source), so a split is offered
only where the boundary is provably inert:

- A boundary is a blank-line-preceded line that is **not absorbable** by the
  construct above it (not indented, not a list/quote/table continuation).
- Fence state is tracked per CommonMark (marker character, run length, no
  trailing info string); boundaries inside fences are never offered, and an
  unterminated trailing fence stays whole in the tail.
- A text containing footnote references or link-reference definitions
  **never splits** — both resolve against later text.
- Below `MIN_SPLIT_CHARS = 600` (or a prefix under `MIN_PREFIX_CHARS = 200`)
  there is nothing to win and the whole text is the tail.

Declining costs speed, never correctness: the no-split result is exactly the
previous whole-text behavior.

### 2.3 Rendering — `frontend/src/components/assistant/blocks/text-block.tsx`

`TextBlock` feeds the block text through `useSmoothReveal`, splits the result,
and renders prefix and tail as two memoized `MarkdownSegment`s (full
react-markdown + GFM + sanitize pipeline for both — the live tail is **not**
plain text, so promotion cannot cause layout jumps). A `"\n"` is restored at
the seam so split output is byte-identical to a whole parse
(`text-block.split.test.tsx` proves DOM equivalence). While streaming, a
blinking caret is glued to the end of the last line.

### 2.4 Invariants

These must survive any change in this document:

1. Settle returns the full text synchronously; Stop/cancel means no further
   typing.
2. Reduced motion disables pacing and renders as it arrives.
3. `SNAP_BACKLOG_CHARS`-sized jumps snap; history and re-mounts never re-type.
4. The final settled render is byte-identical to an unsmoothed render of the
   same text (the terminal invariant — the single most valuable property
   here).
5. Output is always a prefix of arrived text; anything reading "the message"
   (copy, export, retry) reads the transcript model, never the revealed
   prefix.
6. `splitStableMarkdown` declines rather than guesses.

## 3. Residual defects in the baseline

Verified against the code, not inherited from the retired plan:

- **D1 — torn grapheme clusters.** The controller returns
  `text.slice(0, revealed)` on raw UTF-16 offsets
  (`use-smooth-reveal.ts:104`). A cut inside a surrogate pair, ZWJ sequence,
  skin-tone modifier or combining mark paints a replacement glyph or a
  decomposed sequence for a frame or two on essentially every emoji.
- **D2 — inline markdown flicker.** A partially revealed `**bold**`,
  `` `code` `` or `[link](url)` renders as literal source (`**bo`, the raw
  URL) and then re-renders styled when the closer is revealed — a flicker on
  nearly every answer, since emphasis and links are ubiquitous. This happens
  whether the cut is mid-construct because of pacing or because that is all
  that has arrived.
- **D3 — mid-word cuts.** A cut inside a word paints a fragment that reflows
  when the rest lands.
- **D4 — residual stepping.** `rate = max(700, backlog/0.09)` has its only
  equilibrium at an empty buffer: each chunk drains in ~90–170 ms and the
  text then freezes until the next chunk. With typical inter-chunk gaps of
  200–400 ms the text is motionless most of the time — the stepping the
  feature exists to remove, just at a finer grain. (This is the one finding
  from the retired adversarial review that applies to the shipped controller,
  and it is correct: a controller that drains to empty cannot smooth in
  steady state.)

## 4. Delta A — boundary-safe reveal cuts (implemented)

Fixes D1–D3. `frontend/src/lib/assistant/reveal-boundary.ts` chooses safe cuts,
and `useSmoothReveal` applies them with the monotone display clamp described
below. The module and controller integration landed in `1dee68e6` and
`1affd801`.

### 4.1 Contract

`safeRevealEnd(text, desired): number` returns the offset where a reveal of
`desired` UTF-16 units may actually end. It is a **display transform on the
controller's output** — applied when slicing, never fed back into the rate
loop, so it cannot change the reveal rate or strand content. Properties:

- `0 ≤ result ≤ min(desired, text.length)` — the cut only ever moves
  **earlier**.
- Idempotent on safe boundaries: plain prose cuts exactly where asked.
- Settled blocks are unaffected: the transform applies only while paced;
  `active = false` still returns the full text, so invariant 2.4.4 holds.

### 4.2 Rules, in application order

1. **Grapheme safety** (unconditional — a broken glyph is never acceptable).
   The cut moves back to the start of the grapheme containing `desired`, using
   `Intl.Segmenter.segment(text).containing(desired)`. A fixed look-behind
   window is not correct: combining sequences and regional-indicator context
   are not bounded, so a window can begin inside a cluster and report a false
   boundary at its own start. End-of-string is always a boundary. Without
   `Intl.Segmenter`, a fallback walks back over low surrogates and left-attaching
   code points (combining marks, ZWJ, variation selectors, skin-tone modifiers,
   keycap); the fallback is intentionally conservative but cannot implement all
   Unicode grapheme-break rules without the platform segmenter.
2. **Inline-construct safety.** One left-to-right scan over the trailing
   256 units finds the earliest still-open inline construct — emphasis runs of
   **two or more** markers (`**`, `__`, `~~`), backtick code spans,
   `[text](destination)` links — and moves the cut before it. Code spans win
   over everything (no other marker means anything inside one); escaped markers
   are skipped; delimiter runs must be left/right-flanking and close a run of
   the same character and length. A `]` exactly at the reveal head remains
   provisional until one following unit proves whether it starts an inline-link
   destination. Link destinations track balanced, escaped parentheses. A
   shortcut reference `[x]` is released as literal text as soon as a following
   non-`(` unit disambiguates it. Three further rules, each of which the
   phase-2 attack found load-bearing:
   - **Single-marker runs are never held.** A lone `*` or `_` is far more often
     literal prose — a glob (`*.ts`), a multiplication (`5*3`), a list bullet,
     `SELECT *` — than an italic opener that will close, and the costs are
     asymmetric: a false positive freezes the reveal, a false negative flickers
     one asterisk. This also subsumes the old list-bullet special case.
   - **A backtick run of three or more at line start is a fenced BLOCK, not a
     code span.** Fence content is literal code with nothing inline to protect,
     so the scan skips to the closing fence (resetting paragraph-level
     delimiter state) or, if the fence is still open, holds nothing at all.
   - **A delimiter run touching the reveal head stays provisional.** Right-hand
     flanking is undecidable there, and `**` is a single model token, so an
     arrival routinely ends right after one. Closing is still decided normally
     (it depends only on the left context), so a genuine closer closes; a cut
     that would land *inside* an already-arrived run always moves before it.
3. **Word safety.** A cut landing inside a word (letters/digits on both
   sides) snaps back to the word start, bounded by `MAX_WORD_HOLDBACK = 24`
   so long tokens (URLs, identifiers) reveal progressively instead of being
   withheld. Scripts written without inter-word spaces (Han, kana, Hangul,
   Thai, Lao, Khmer, Myanmar) are exempt — per-character reveal is the
   correct granularity there.

### 4.3 The bound and the trade

Rules 2–3 together may withhold at most `MAX_HOLDBACK = 96` units of the paced
reveal head; past that the hold is **clamped, not abandoned** — the cut rides a
constant 96 units behind instead of snapping forward. This is the explicit
answer to the retired review's core objection ("there is no rule that both never
shows broken markup and never withholds output indefinitely"): a model can emit
an opener it never closes, and a stalled answer is worse than a stray asterisk,
so the bound is the named trade.

Clamping rather than abandoning matters, and the phase-2 attack found the
original abandon-at-the-bound behaviour to be a smoothness regression in its own
right: abandoning returns the full cut in one frame, so a never-closed opener
produced a ~96-character freeze followed by a ~96-character jump — a worse
stutter than the flicker being prevented. Clamping keeps one revealed unit
moving out for every unit in.

The bound is in *revealed* units, not arrived units and not wall-clock time.
Because the paced head lags arrival by roughly one chunk under §5, an opener is
released somewhat later than 96 further arrivals; and if the producer pauses
immediately after a short opener, the short tail remains withheld until more
text arrives or settle synchronously exposes the full text.

### 4.4 Accepted approximations

The inline scan is parser-lite, not CommonMark: it implements flanking and
same-run matching but not the full delimiter multiple-of-three rule, link
titles, or every CommonMark destination edge case. It also does not hold
single-marker emphasis at all (above), so `*italic*` flickers one asterisk
before it resolves. Its bounded window does not carry block-fence state from an
opener more than 256 units behind the head, so a literal `` ` `` or `**` late in
a long fenced block can still cause an unnecessary hold. Every such hold is
bounded by `MAX_HOLDBACK` revealed units and never changes settled output; it is
not promised to be transient in wall-clock time while a producer is paused. This
is deliberate; a real incremental CommonMark parser here is not worth its
complexity.

Also deliberately **not** done: the one-grapheme provisional-tail reserve
(holding the last revealed grapheme in case a later chunk extends it with a
modifier). When the reveal head has caught up to arrival, a modifier arriving
later can still mutate the last painted glyph — but the unsmoothed baseline
has exactly the same behavior, so this is not a regression, and the reserve
machinery isn't worth it.

### 4.5 Integration

Applied inside `useSmoothReveal`'s paced return path:
`text.slice(0, safeRevealEnd(text, min(revealed, text.length)))`, plus a
**monotone display clamp**: the returned cut never falls below the previous
frame's cut along a prefix-preserving append chain, so appended text can never
retract already-painted characters (a construct opening near the head, or the
256-unit scan window sliding, could otherwise move the safe cut earlier for a
frame). The clamp stores the prior source text and cut in guarded render-time
React state, not refs (render-time ref access violates React 19 purity), and
resets whenever the next source is not an append of that text. A length-only
reset misses same-length replacements and a shrink-plus-grow coalesced into one
React commit, allowing an old cut to leak into unrelated replacement content.
`TextBlock` does not change.

### 4.6 Amendments from the phase-1 attack

- Replaced the unsound ±64-unit segmentation claim with `Segments.containing`;
  an arbitrarily long combining sequence is the counterexample.
- Tightened inline delimiter and link handling. In particular, `**bold*` and a
  chunk ending exactly at `[label]` were released by the staged scanner and
  would still flicker when the final marker or `(` arrived. Delimiter runs are
  also capped at the desired head so arrived-but-not-yet-revealed markers cannot
  make an incomplete painted run appear closed.
- Clarified that the 96-unit bound is an arrival-progress bound, not a time
  bound, and documented the deliberate long-fence disagreement with the block
  splitter.
- Changed clamp reset identity from length to prefix continuity. The monotone
  claim is valid only on an append chain; replacements must establish a new
  chain even when their length does not shrink.
- Replaced the proposed render-time refs with guarded state adjustment after
  the React 19 lint gate correctly rejected reading and mutating refs in render.

### 4.7 Amendments from the phase-2 attack

Four defects were found by driving the hook with realistic feeds rather than by
reading the code, and fixed in `fix(assistant): stop the boundary transform
stalling ordinary answers`:

- **Fenced code blocks froze.** ``` was scanned as an unterminated inline code
  span, so the reveal parked at the top of every code block for a whole
  holdback and then painted ~100 characters in one frame. Fences are now
  recognized as block constructs.
- **A lone `*` stranded short answers.** `"Use *.ts to match TypeScript
  files."` painted as `"Use *"` for its entire stream and only appeared on
  settle, because the block never grew 96 units past the marker. Single-marker
  runs are no longer held.
- **A chunk ending exactly at `**` painted literal markers**, and the monotone
  clamp then pinned them for the rest of the turn — so the headline D2 fix did
  not hold at the single most common chunk boundary. Head-touching runs are now
  provisional, and a cut is never placed inside an already-arrived run.
- **Abandoning the hold at the bound was itself a stutter** (freeze then jump);
  the hold is now clamped instead. See §4.3.

Also corrected in this document: the holdback bound is in *revealed* units, not
arrived units.

## 5. Delta B — adaptive drain spread (implemented)

Fixes D4. It is the only change that addresses the residual stepping, and its
added latency is bounded by one observed inter-arrival gap — precisely the
time the baseline rendering spends frozen. The guards below make it inert for
single-burst content and separable for revert. It landed in `41678074`.

### 5.1 Control law

Track inter-arrival gaps and spread each chunk over roughly the interval
until the next one arrives, instead of draining in a fixed 90 ms:

```
// on an observed committed append, with now = performance.now():
gap = now − lastArrival
if 0 < gap ≤ STALL_GAP_MS: gapEma = (gapEma == null) ? gap
                                  : gapEma + GAP_EMA_ALPHA × (gap − gapEma)
if gap > STALL_GAP_MS:    gapEma = null
lastArrival = now
// source is not a prefix append (replacement): clear both arrival refs

// per frame (after the unchanged SNAP_BACKLOG_CHARS check, which runs first):
spread   = clamp(gapEma, DRAIN_MS, MAX_SPREAD_MS)           // ms
adaptive = gapEma != null  &&  now − lastArrival ≤ IDLE_GAPS × spread
rate     = adaptive
  ? max(MIN_ADAPTIVE_CHARS_PER_SECOND, backlog / (spread/1000))
  : max(MIN_CHARS_PER_SECOND,          backlog / (DRAIN_MS/1000))   // legacy

// adaptive branch only: preserve fractional character credit across frames
credit  += rate × min(frameSeconds, MAX_FRAME_SECONDS)
advance  = floor(credit); credit -= advance
```

| Constant | Value | Why |
| --- | --- | --- |
| `GAP_EMA_ALPHA` | 0.3 | Per-arrival EMA (RTT-estimator style); updates are per event, not per frame, so refresh-rate independence is not at stake |
| `MAX_SPREAD_MS` | 400 | Hard bound on added display lag |
| `MIN_ADAPTIVE_CHARS_PER_SECOND` | 60 | ~1 char/frame at 60 Hz — still visible motion; the 700 floor stays only in legacy mode, because at 700 cps a typical 50-char chunk drains in 70 ms and the spread never engages |
| `STALL_GAP_MS` | 1500 | A gap this long is a tool call or a stall, not network jitter; it must not inflate the EMA |
| `IDLE_GAPS` | 2 | Producer idle for > 2 spreads → revert to the legacy fast drain so the answer converges promptly |

### 5.2 Guards, exactly

- **Fewer than two arrivals → legacy law verbatim.** `gapEma` needs a gap,
  a gap needs two separately committed prefix-appends. Batched growth that is
  visible to React as one commit is one observation and deliberately leaves
  the legacy law armed. Every existing `use-smooth-reveal` test drives at most
  one observed growth event (the replacement case starts non-empty and then
  shrinks), so the whole suite must pass unmodified — that is the acceptance
  check, not a hope.
- **Snap unchanged and checked first.** A projection-sized jump snaps even
  with an armed `gapEma`.
- **Settle unchanged.** `active = false` returns the full text synchronously;
  Delta B never delays stream end, because settle bypasses pacing entirely.
- **Idle revert.** When arrivals stop mid-stream (tool call), motion decays
  to the 60 cps floor and, after `IDLE_GAPS × spread` (≤ 800 ms), the legacy
  fast drain flushes the remainder.
- **Refresh-rate independence.** Adaptive advances retain fractional credit.
  `max(1, round(rate × seconds))` is not valid at a 60 cps floor: once the
  product drops below one, it reveals one character per display frame (60 cps
  at 60 Hz, 144 cps at 144 Hz) instead of one character per elapsed time.

### 5.3 Latency budget and revert criterion

Worst-case added display lag versus today is `MAX_SPREAD_MS − DRAIN_MS ≈
310 ms`, and only when observed gaps are actually that long — i.e. exactly
when today's rendering is frozen for ~78% of every gap. In return, motion is
continuous through the gap. This is the jitter-buffer positive-lag setpoint
the retired review demanded, implemented as a spread denominator rather than
a full playout controller. If QA or the product owner judges the trailing
feel worse than the stepping, Delta B reverts by deleting its constants and
the `adaptive` branch; Delta A is unaffected.

### 5.4 Amendments from the phase-1 attack

- Defined an arrival as an observed committed prefix-append. Coalesced updates
  cannot yield an inter-arrival sample and therefore remain in legacy mode.
- A gap over `STALL_GAP_MS` now clears the old EMA rather than merely refusing
  to update it; otherwise the first post-stall chunk would reuse stale cadence.
- Added fractional adaptive credit so the 60 cps floor remains time-derived at
  high-refresh rates and does not violate the baseline controller guarantee.
- Replacements are detected by prefix continuity rather than length alone and
  reset the cadence history as well as the display clamp.

## 6. Implementation plan

For a single implementing agent, executed cold, in order. Spec (§4–§5) wins
over this plan on any conflict; report conflicts, do not silently resolve.

Ground rules:

- Branch: `docs/smooth-text-streaming-plan` in this worktree (PR #1394 into
  `rollup-chat-2026-08-07` is already open; push updates to the same branch
  only when instructed). Commit per work package with the message given.
- Files outside the per-WP lists are out of scope. **No dependency or
  lockfile changes** (Wizard Bundle Freshness CI trips on any).
- `frontend/node_modules` in this worktree is currently a **symlink** into
  the sibling worktree `tidy-ocean` (identical `package-lock.json`, verified
  by hash). It works for vitest/tsc/eslint. For a pristine environment:
  remove the symlink and run `npm ci` in `frontend/`.
- Existing tests are a hard invariant and may not be edited to pass:
  `use-smooth-reveal.test.tsx` (7 tests), `markdown-stream.test.ts`,
  `text-block.test.tsx`, `text-block.split.test.tsx`.

### WP1 — land the boundary module

Files: `frontend/src/lib/assistant/reveal-boundary.ts`,
`frontend/src/lib/assistant/reveal-boundary.test.ts`.

Implemented in `1dee68e6` after applying §4.6: `Segments.containing`, flanking
and same-length emphasis runs, provisional head-ending `]`, balanced link
destination parentheses, and the corresponding adversarial module tests.

Acceptance: the expanded `reveal-boundary.test.ts` suite and eslint on both
files are clean.

Commit: `feat(assistant): add boundary-safe reveal cut for streamed text`

### WP2 — apply the cut in the controller

Files: `frontend/src/hooks/use-smooth-reveal.ts`,
`frontend/src/hooks/use-smooth-reveal.test.tsx` (additions only).

Change: per §4.5 — import `safeRevealEnd`, apply it in the paced return path,
and add the monotone display clamp as guarded render-time state (last cut plus
last source; reset when source continuity breaks). No controller-rate changes
in this WP.

New tests (concrete; drive with the existing fake-timer/`runFrames` harness):

- Torn-emphasis: stream `"make it **bold** please"` arriving as
  `"make it **bo"` then `"ld** please"`; assert every painted output either
  omits the opening run or contains a complete `**...**` run. The former regex
  `/\*\*[^*]*$/` is vacuous for `**bold*`, the exact one-marker-short frame the
  test must reject.
- Grapheme: assert no frame's output ends with a lone high surrogate
  (`/[\uD800-\uDBFF]$/`), a ZWJ, a variation selector or a skin-tone modifier.
  The fixture must be long and emoji-dense and must be fed in chunks: the
  originally-shipped `"Hi 👩‍💻!"` converged inside the first frame's
  11-character advance, so no cut ever landed mid-cluster and the test passed
  unchanged against the pre-Delta-A hook. Corrected in phase 2; grapheme safety
  is real (an unprotected slice tears 12 surrogates and 3 ZWJ sequences over 90
  frames of the corrected fixture) but the original test did not show it.
- Monotonicity: use a 301-unit fixture whose opening backtick is exactly about
  to slide out of the 256-unit scan window while its old closer remains within
  the 96-unit holdback. Converge at 301 units, append one unit, and assert every
  painted output is a prefix of the next. This fails without the clamp; the
  torn-emphasis feed does not reliably exercise a decreasing safe cut.
- Replacement reset: replace a fully painted source with a same-length source
  containing an open emphasis run; assert the new unsafe tail is withheld.
  This fails if clamp identity is based only on length.
- Settle override: with a withheld `**bol` tail, flip `active` to false;
  assert the full literal text (including `**bol`) is returned immediately.

Acceptance: full existing `use-smooth-reveal.test.tsx` suite green untouched,
plus the new cases; `text-block` suites green (no changes expected there).

Commit: `feat(assistant): cut streamed reveals at safe boundaries`

### WP3 — adaptive drain spread

Files: `frontend/src/hooks/use-smooth-reveal.ts`,
`frontend/src/hooks/use-smooth-reveal.test.tsx` (additions only).

Change: exactly §5.1–§5.2. Arrival tracking shares the target-update effect but
observes the full source plus paced state so it can reject replacements;
`gapEma`/`lastArrival` are refs (never state), and the frame step already
receives `now`.

New tests:

- Spreading: feed 60-char chunks at 320 ms intervals (20 frames apart) for
  three arrivals; 8 frames (128 ms) after the third arrival assert the output
  is still short of the arrived text — the legacy law would have converged by
  frame 6 (60 chars at 700 cps ≈ 86 ms) — and that it is longer than it was
  2 frames after the arrival (still advancing, not frozen).
- Idle revert: arm a 320 ms EMA, feed a sub-snap large backlog, compare frame
  advances immediately before and after `2 × spread`, and assert the legacy
  advance is larger before checking convergence. A convergence deadline alone
  also passes today's always-legacy controller and does not prove the branch.
- Snap with armed EMA: two chunks 300 ms apart, then a +4000-char jump;
  assert the very next frame shows the full text.
- Stall reset: arm the EMA, pause for more than `STALL_GAP_MS`, append a small
  chunk, and assert it uses the legacy drain rather than stale adaptive cadence.
- High-refresh floor: drive rAF at 8 ms, reach the 60 cps floor, and assert
  progress follows accumulated elapsed-time credit instead of frame count.
- Single burst unchanged: the existing "withholds arrived characters",
  "log-time convergence", "snap", "replacement", "settle" and
  "reduced-motion" tests pass byte-for-byte unmodified — this is the
  fallback-guard acceptance, and it is the whole reason the guard exists.

Acceptance: full suite green; manually eyeball one streamed answer in the dev
app if a running stack is available (optional, not gating).

Commit: `perf(assistant): spread streamed reveals over the arrival cadence`

### WP4 — flip this document's status markers

Files: `docs/chat/smooth-streaming.md`, `docs/chat/README.md` (only if the
one-line index entry needs rewording).

Change: §4 header "(proposed; module staged)" → "(implemented)"; §5
"(proposed)" → "(implemented)"; opening status paragraph updated; add the
implementing commit range.

Commit: `docs(chat): mark smooth-streaming deltas implemented`

### 6.1 Amendments from the phase-1 attack

WP1 is no longer a commit-as-staged package because its scanner and grapheme
window had correctness counterexamples. WP2's original emphasis regex and
torn-emphasis monotonicity fixture did not prove their claims, and WP3's
original idle deadline did not distinguish idle reversion from an
always-legacy controller. The package descriptions above contain the corrected
implementation and differential assertions.

### Verification commands

From `frontend/`:

```
npm run test          # NODE_ENV=test vitest run — the unit gate
npm run lint          # eslint .
npm run build         # tsc -b + vite builds — THE CI gate
```

CI runs `npm run build`, i.e. `tsc -b` with `noUncheckedIndexedAccess`;
`tsc --noEmit` alone misses errors CI catches — never substitute it. Vitest
flakes under machine load or when a live dev server owns :3000/:3001 (it
answers test fetches with real 401s); re-run with `--no-file-parallelism`
and check `lsof -ti :3000 :3001` before trusting a red run.

### Must not change

The §2.4 invariants; every existing test named in the ground rules; the
public signature `useSmoothReveal(text, active): string`; `TextBlock` props;
`splitStableMarkdown` and its tests (untouched by both deltas); the caret
markup; `SNAP_BACKLOG_CHARS`, `DRAIN_MS`, `MIN_CHARS_PER_SECOND`,
`MAX_FRAME_SECONDS` values in legacy mode.

### 6.2 Known residuals (phase-2 attack, accepted)

- **`safeRevealEnd` is O(text length) per frame.** `Intl.Segmenter`'s
  `Segments.containing` scans rather than seeking: measured 4 µs at 1 k
  characters, 27 µs at 20 k, 87 µs at 50 k. The monotone clamp's render-phase
  `setDisplayed` roughly doubles component invocations per advancing frame
  (measured 37 renders over 20 frames), so budget ~2× those numbers plus a
  second `splitStableMarkdown` pass. At 50 k characters that is ~1 % of a 60 Hz
  frame — not a frame-budget regression, but it is the one place the delta
  reintroduces the per-paint linearity §2.2 exists to remove. If a 200 k-answer
  profile ever shows it, the sound fix is to segment only the suffix beginning
  at the previous frame's cut, which is already a known grapheme boundary in the
  same string.
- **The adaptive credit ref is mutated inside the `setRevealed` updater**, i.e.
  during render. This is the same render-phase side effect §4.6 removed from the
  display clamp. Under StrictMode double-invocation or a discarded concurrent
  render the credit is consumed twice, so the adaptive branch can advance
  slightly faster than elapsed time; measured effect is ~2 characters over 40
  frames and it is bounded by `target`, so it is a purity defect rather than a
  visible one. Fixing it properly means moving the credit and a revealed mirror
  into the rAF effect closure.
- **`prefersReducedMotion()` calls `matchMedia` on every render** (pre-existing,
  now at ~2 renders/frame) and does not subscribe, so toggling the OS setting
  mid-stream has no effect until the next render.

### Deliberately deferred

- **Per-group CSS fade (the retired plan's subsystem D).** Requires per-word
  wrapper spans through the react-markdown pipeline — invasive for
  selection/copy/find-in-page and screen-reader buffers — and its value drops
  sharply once motion is continuous (Delta B). Revisit only if the result
  still reads stepped.
- **Scroll anchoring / stick-to-bottom.** A real concern, but an independent
  surface with its own state; not part of the text-reveal pipeline.
- **Replay rig, telemetry, factorial perception study.** Right-sized for a
  platform team, not for this delta; the correctness properties it wanted are
  landed as unit tests instead (terminal invariant, split byte-identity,
  boundary properties).
- **Event-tape content model.** The transcript already has a typed
  block/stream model (see `03-stream-protocol.md`); pacing is per text block
  and needs nothing more.
- **Word-granularity reveal grouping via `Intl.Segmenter` word segmentation,
  server-side smoothing, provisional-tail grapheme reserve.** See §4.4 and
  the retired plan's own client-side decision; none earn their complexity
  here.

## 7. Disposition of the retired plan and review

Kept (as design rationale above): the empty-buffer-equilibrium finding (→ D4,
§5); UTF-16 slice grapheme breakage (→ D1, §4.2.1); holdback needs an
explicit bound and named fallback (→ §4.3); reduced-motion is non-negotiable
and the terminal invariant is the most valuable test (→ §2.4, already
implemented); markdown non-prefix-stability (→ already the reason §2.2 is
conservative); time-derived-not-frame-derived pacing (→ already implemented);
Stop must stop instantly (→ settle hand-over, already implemented).

Dropped, with reasons: the four-subsystem architecture, jitter-buffer
controller with adaptive `L*`, event tape, atom/word unit contract, commit
horizon, fade layer, measurement program, and a11y matrix — they specify a
ground-up streaming platform, while the shipped baseline already solves the
arrival-burstiness and render-cost problems well enough that only D1–D4
remain; the delta above fixes those at
~1% of the proposed surface. The plan's critique of the Upstash reference
implementation (frame-derived rate, unbounded lag) was correct but aimed at
code this repo never shipped.
