# Smooth Text Streaming — Concepts & Plan

**Status:** Draft v2 — concepts only. No implementation, no code, no repo changes proposed.
**Date:** 2026-08-07
**Companion:** `smooth-streaming-plan.review.md` (adversarial review of v1 by Codex Sol; v2 is the response to it)

---

## 0. TL;DR

The idea worth copying from [Upstash's *Smooth Text Streaming*](https://upstash.com/blog/smooth-streaming) is **decoupling the network stream from the visual stream**. The article's implementation of that idea is not usable as written — its reveal rate is a function of the user's monitor, and its lag is unbounded.

v1 of this plan copied the decoupling and replaced the article's constant-rate reveal with a backlog-draining controller. Adversarial review showed that controller was **mathematically incapable of doing its job**: its only equilibrium is an empty buffer, which is precisely the state in which no smoothing happens.

v2 replaces it with the correct prior art — an **adaptive playout buffer** (the jitter-buffer model from real-time media), which has a *positive* latency setpoint and is therefore stable in exactly the state we want.

Four subsystems, in dependency order:

| | Subsystem | Fixes |
|---|---|---|
| **A** | Content model — typed, append-only event tape | Makes everything else expressible |
| **B** | Playout layer — adaptive jitter buffer over the tape | Arrival burstiness, granularity |
| **C** | Render layer — parser-aware commit horizon | Re-parse cost, layout churn |
| **D** | Perceptual layer — bounded per-atom CSS transition | Perceived smoothness |

**A is a prerequisite for B, C, and D. B, C, and D are not independent of each other** (v1 wrongly claimed they were — see §9.2).

---

## 1. Problem decomposition

"Streaming looks janky" is four problems that get conflated. Fixing one and shipping produces a marginal improvement and a reverted feature.

| # | Problem | Symptom | Layer |
|---|---|---|---|
| P1 | **Arrival jitter** | A paragraph lands in one frame, then 400 ms of nothing | Transport |
| P2 | **Granularity** | Reveal unit is a ragged provider token (`" implemen"` / `"tation"`) | Transport |
| P3 | **Render cost** | Every chunk re-parses the whole markdown tree; frames drop; the message flashes | Client render |
| P4 | **Layout instability** | Content jumps, autoscroll fights the user, code re-highlights and flickers | Layout / scroll |

The Upstash article addresses P1 and P2 only. On a three-line answer that suffices. On a 2,000-token answer with three code blocks, P3/P4 dominate — and a 60 Hz typewriter makes them *worse*, converting ~10 renders/sec into ~60.

**Consequence:** do not treat this as one feature. Treat it as four, measure each, and ship only the ones that earn it (§10).

---

## 2. Analysis of the source article

### 2.1 The algorithm

```
typewriterSpeed = 5   // ms per character
fullText = parts.join('')

onAnimationFrame(time):
  if (time - lastTime) > typewriterSpeed:
    streamIndex += 1
    stream = fullText.slice(0, streamIndex)
    lastTime = time
  if streamIndex < fullText.length: requestAnimationFrame(...)
  else: done
```

### 2.2 Defect A — the reveal rate is set by the user's monitor, not by the constant

The cursor advances by **exactly one UTF-16 code unit per `requestAnimationFrame` callback that passes the gate**. It does not accumulate elapsed time and it does not reveal multiple units. Because rAF fires at the display refresh interval (~16.7 ms at 60 Hz) and 16.7 > 5, the gate passes on *every* frame.

| Display | Actual reveal rate |
|---|---|
| 60 Hz | ~60 units/sec |
| 120 Hz (ProMotion) | ~120 units/sec |
| 144 Hz | ~144 units/sec |
| Background tab | rAF is **paused** in most browsers (not throttled to a predictable cadence) — never rely on any rate |

The article states this yields ~200 chars/sec. It does not, on any common display. Precisely: the rate is **quantised by callback cadence**; ~200 would require a ~5 ms callback interval, i.e. a ~200 Hz panel. The correct statement is not "200 is impossible" but "**the rate is uncontrolled and hardware-dependent**." On a stable 60 Hz cadence, any `typewriterSpeed` below ~16.7 is inert — 1, 3 and 5 produce identical output, which is why "5 works really well for me" is unfalsifiable.

**Fix:** rate must be **time-derived**, never frame-derived — compute atoms-owed from elapsed wall-clock and carry the remainder.

### 2.3 Defect B — unbounded and growing display lag

Held rates are model-, provider-, tier-, language- and proxy-dependent, so we will not assert an envelope here; it must come from the replay corpus (§8). For scale only, published provider medians for a current frontier model sit near 80–90 output tokens/sec, which for typical English is on the order of a few hundred characters/sec — well above 60.

*Conditional example:* if 4,000 characters arrive over 15 s and the reveal runs at 60/sec, ~900 are shown during arrival and the remaining ~3,100 take a further **~52 seconds**. The user watches a typewriter for most of a minute after the network finished, and pressing Stop changes nothing visible because the backlog is local.

This is the failure mode that gets the feature reverted. Every **fixed-rate** design has it, including the 30 ms-per-character `StreamBuffer` pattern (~33/sec, worse).

### 2.4 Defect C — no terminal semantics

The loop has no notion of stream end, cancel, error, tab-switch, or navigation. §7.

### 2.5 Defect D — `slice` cuts UTF-16 code units

JavaScript string indices are UTF-16 code-unit indices. A raw `slice(0, i)` can expose a lone surrogate and can split a multi-code-point grapheme: emoji modifiers, skin tones, flags (regional-indicator pairs), ZWJ sequences (👩‍💻), and base-plus-combining-mark sequences. Symptom is a replacement glyph or a decomposed sequence for one frame, then a snap.

**Fix:** the reveal atom is a **grapheme cluster**, not a code unit — *plus* a provisional-tail rule, because a segmenter alone is insufficient (§4.4).

---

## 3. Subsystem A — content model

**v1 used a flat `fullText` string. That is not expressible enough and must be replaced before anything else is designed.**

A modern turn is not a string. It interleaves text deltas, reasoning, tool-call lifecycle (start → argument deltas → ready → approval → result → error → retry), citations anchored to text ranges, cards, and file attachments. A flat string cannot represent ordering, identity, or anchoring for any of these.

**The model is an append-only typed event tape:**

- Each entry: `{ seq, type, payload, arrivalTime, epoch }`.
- Every entry has a **stable id**; lifecycle updates reference the id rather than appending a new logical item.
- Text entries additionally carry their offset range in the derived text.
- The **cursor is a position on the tape**, not an index into a string.

Three properties this buys, none of which the flat model has:

1. **Ordering is explicit**, so §7.6's event-barrier semantics become expressible.
2. **`arrivalTime` per entry** is what the playout layer (§4) needs; a string has nowhere to put it.
3. **`epoch`** (a per-turn generation id) makes cancel races decidable — see §7.3.

### 3.1 Units — one canonical definition

v1 mixed grapheme indices, UTF-16 `.length`, words, and "chars per second" in a single control loop. Subtracting graphemes from code units is meaningless. The contract:

| Concept | Unit | Used for |
|---|---|---|
| **Atom** | one grapheme cluster (`Intl.Segmenter`, `granularity: 'grapheme'`) | the canonical position and pacing unit |
| **Slice offset** | UTF-16 code-unit index | derived from atom index; used *only* to slice for rendering |
| **Reveal group** | 1..N atoms released together (may be a word) | animation keying and DOM batching |
| **Rate** | **atoms/sec** | the controller's only rate unit |

A word is a *grouping of atoms*, never a substitute unit. Revealing a 35-atom URL costs 35 atoms of budget; if the frame's budget is smaller, the group is deferred to the next frame rather than splitting the word or blowing the budget. This is stated so that variable-length atoms cannot produce the rate swings v1 would have had.

### 3.2 Transport decoding boundaries

Bytes → text is not free and must be specified, or the atom model is built on sand:

- Decode UTF-8 **incrementally with a streaming decoder** (`TextDecoder({stream:true})`); a provider chunk can split a multi-byte sequence.
- SSE record boundaries, JSON delta boundaries, and grapheme boundaries are all different. Segment only after decoding, never on raw chunk boundaries.
- Reconnect/resume must be idempotent — dedupe by `seq`, never by content.

### 3.3 Non-append updates

Prefix-only growth is an **assumption, not an invariant**. Providers and client libraries can replace a snapshot, reconcile a tool part in place, redact reasoning, retry a step, or switch branches. The tape must define a fallback: on any non-append mutation, **invalidate the cursor to the mutation point and re-derive**, rather than silently rendering a divergent view. This must be detected, not assumed away.

---

## 4. Subsystem B — playout layer

### 4.1 The correct model is a jitter buffer

The problem — a bursty producer, a consumer that must present at a smooth rate, and a buffer in between — is the **playout buffer** problem from real-time audio/video. That prior art should be used directly.

The defining property, and the one v1 got wrong: **the latency setpoint is positive.** The buffer exists to hold content. A controller that drives the buffer to empty has removed the mechanism that absorbs jitter.

### 4.2 Why v1's controller could not work

v1 proposed `rate = p̂ × 1.10 + backlog / T`. With producer rate `p` and backlog `b`:

```
db/dt = p − rate = p − 1.1p − b/T = −0.1p − b/T   <   0   for all b ≥ 0
```

Backlog is **strictly decreasing everywhere**. The only equilibrium is `b = 0` — a starved consumer revealing each atom the instant it arrives, i.e. exactly the un-smoothed behaviour the feature exists to prevent. In steady state v1 would have done nothing, and in burst it would have sawtoothed. It is not a tuning problem; the law has no positive fixed point.

### 4.3 The v2 control law

Express buffer occupancy in **time**, not atoms, and hold it at a positive target:

```
lag        = backlog_atoms / rate_est          // seconds of content held
rate       = rate_est × (1 + K × (lag − L*))   // atoms/sec
rate       = clamp(rate, RATE_MIN, RATE_MAX)
```

Linearising with `rate_est ≈ p`:

```
db/dt = −K × (b − p·L*)
```

A first-order system with a **stable equilibrium at `lag = L*`** and time constant `1/K`. Bursts decay exponentially back to the setpoint instead of oscillating; steady state reveals at exactly the producer's rate, shifted by `L*` — which means **in steady state the pacing is invisible**, which is the goal (§11).

### 4.4 Adapting `L*`

Fixed `L*` is wrong for the same reason fixed rate is: jitter varies by model, network and content. Size the buffer to the jitter actually observed.

- Track the distribution of **inter-arrival gaps** over a sliding window.
- `L* ← clamp(p95(gaps), L_MIN, L_MAX)`.
- **Grow fast, shrink slow** (standard jitter-buffer asymmetry): an underrun raises `L*` immediately; decay toward the floor gradually (e.g. 5 ms/s) so recovery does not re-trigger underruns.
- All EMAs are specified by a **wall-clock time constant**, with the per-update coefficient derived as `α = 1 − exp(−Δt/τ)`. v1's "α = 0.15 per frame" was frame-derived and would have responded twice as fast at 120 Hz — the exact error §2.2 diagnoses in the source article.

### 4.5 Producer stall (the 8-second tool call)

Explicitly specified, because v1 left it undefined:

- `rate_est` **decays on wall-clock time**, not only on arrivals, so it cannot go stale-high across a stall and dump the next burst.
- A stall longer than `STALL_THRESHOLD` (~1 s) puts the controller in **idle**: `rate_est` is held (not decayed to zero), the reveal loop is descheduled, and no rAF runs.
- On resumption, the controller **re-enters via the ramp**, not at the last rate: `L*` is re-primed from the first post-stall gaps.
- Stalls are excluded from the inter-arrival gap distribution — a tool call is not network jitter, and letting it inflate `L*` would add seconds of latency to the rest of the answer.

### 4.6 Terminal behaviour — thresholds derived, not chosen

v1's constants were internally contradictory: `RATE_MAX × TAIL_DEADLINE` bounded the drainable remainder at ~160 atoms while `HARD_LAG_CEILING` permitted 1,500, so for any remainder in between, some stated bound had to be violated. v2 derives the threshold instead of picking it:

```
SNAP_THRESHOLD := RATE_MAX × TAIL_DEADLINE
```

On `finished`: if `backlog ≤ SNAP_THRESHOLD`, drain within `TAIL_DEADLINE`; otherwise **snap**. Coherent by construction.

Note this threshold is rarely reached in a healthy stream: backlog at stream end is ≈ `p × L*`, which for a fast producer and `L* = 250 ms` is well under the threshold. Snapping is the pathological path, not the normal one.

### 4.7 Starting constants

Hypotheses to be falsified by §8, not defaults to ship.

| Constant | Start | Rationale |
|---|---|---|
| `L*` initial | 250 ms | Below perceptual-lag threshold; ~1 typical inter-chunk gap |
| `L_MIN` / `L_MAX` | 80 ms / 600 ms | Floor keeps a buffer; ceiling caps added latency |
| `K` | 2.0 /s | ~500 ms settling; slower than a burst, faster than a sentence |
| `RATE_MIN` | 20 atoms/s | Below this reads as artificially slow |
| `RATE_MAX` | 500 atoms/s | Above this the eye cannot track it; it is an instant paint |
| `TAIL_DEADLINE` | 400 ms | Bounds post-network wait |
| `SNAP_THRESHOLD` | *derived* = 200 atoms | Never set by hand |
| `STALL_THRESHOLD` | 1 s | Separates tool latency from network jitter |

### 4.8 Segmentation and the provisional tail

`Intl.Segmenter` is the right primitive but **is not by itself sufficient for a growing buffer**. It segments the string you give it; the **final segment is provisional**. A later chunk can extend the last already-revealed atom with a combining mark, variation selector, skin-tone modifier, regional indicator, or ZWJ continuation. A monotonic cursor cannot retract it.

**Rule:** never reveal the final segment of an unterminated buffer. Hold one atom (and, at word grouping, one word) in reserve; release it when a following atom confirms the boundary, or at stream end. The reserve is one atom deep, so its latency cost is negligible.

Locale, availability fallback, mixed-script handling, and normalisation form the **Unicode streaming contract** and must be written down before implementation, not discovered in it.

### 4.9 Segmentation for non-space-delimited scripts

Whitespace splitting is wrong, but v1's explanation was wrong too. Precisely:

- **No usable inter-word delimiter:** Japanese, Chinese, Thai, Lao, Khmer, Myanmar. Whitespace splitting degenerates toward whole-sentence chunks.
- **Spaces exist but are not the lexical boundary:** Korean (spaced eojeol), Vietnamese (spaced syllables). Splitting produces *linguistically wrong* chunks, not giant ones.

Both are fixed by `Intl.Segmenter` with `granularity: 'word'` and the correct locale — this is also what AI SDK's own documentation recommends over its default `chunking: 'word'`, which it explicitly flags as poor for these languages.

### 4.10 Where the playout layer runs

| | Server-side (e.g. AI SDK `smoothStream`, `delayInMs: 10`, `chunking: 'word'`) | Client-side |
|---|---|---|
| Smooths arrival jitter | Yes | Yes |
| Fixes render cost (P3) | No | Yes |
| Knows backlog / can adapt `L*` | No | Yes |
| Knows refresh rate, visibility, reduced-motion | No | Yes |
| Correct on history replay / reconnect | No — replays or re-paces old turns | Yes |
| Cost | Holds the connection open longer; spends real wall-clock latency | Free |

**Decision: client-side.** The server emits as fast as it can.

*Correction to v1:* v1 claimed server-side smoothing "compounds per proxy hop." That is wrong — a single transform delays once regardless of how many passive hops carry the stream; compounding requires *multiple independently smoothing* hops. The client-side decision stands on the other five rows, which are sufficient. Server-side smoothing does have one genuine advantage worth noting: it reduces client update frequency for thin clients.

---

## 5. Subsystem C — render layer

At 60 reveals/sec a naive `<Markdown>{visibleText}</Markdown>` re-parses the entire message 60 times per second. On a long answer that, not arrival jitter, is the jank.

### 5.1 The stability assumption v1 got wrong

v1 asserted that a monotonically growing prefix means "no re-flow of already-painted text, ever," and proposed committing paragraphs permanently. **Markdown is not a prefix-stable language.** Counterexamples, all real:

| Construct | Retroactive effect |
|---|---|
| Setext heading | A trailing `===` turns the *preceding* paragraph into an `<h1>` |
| Link-reference definition | A definition near the end resolves `[label][id]` near the beginning |
| Table | The first row is a paragraph until the delimiter row arrives; every later row joins one container |
| List tightness | A later blank line switches an entire list from tight to loose |
| Fenced block | Blank lines inside a fence are content, not block boundaries |
| Lazy continuation | Blockquote/list continuation reinterprets following lines |

Prefix monotonicity guarantees only **append-only source identity**. It does not freeze layout and it does not freeze parse semantics.

### 5.2 Commit horizon, not permanent commit

Replace "committed forever" with a **parser-supplied invalidation contract**:

1. **Name the dialect** (CommonMark + an explicitly enumerated GFM subset: tables, strikethrough, task lists; plus whatever math/footnote extensions are actually enabled). Block stability differs materially per dialect.
2. A block may be memoised only when the parser reports it **closed** *and* no pending nonlocal construct can reach it — no open reference definition, no possible setext underline, no pending table delimiter, no undetermined list tightness.
3. Maintain an explicit **mutable horizon**: the last *K* blocks stay re-renderable. `K` is derived from the maximum backward reach of the enabled constructs, not guessed.
4. Reference definitions have unbounded reach. Either **disable the feature**, or accept a full re-render when one resolves, and say which. Silence is not an option.

### 5.3 Code fences

v1 proposed counting ``` markers and checking parity. That is not what CommonMark specifies: a fence has a marker character, an opening length, and indentation rules, and a closing fence must use the same marker at ≥ the opening length. Backticks inside content do not close anything. **Defer to the parser's block state.** Do not highlight an open fence — the tokenizer's guess changes every frame and flickers.

### 5.4 The live tail must not be plain text

v1 proposed rendering the in-progress block as a raw `<span>` and promoting it on completion. This trades parse jank for **layout jank**: a raw span has none of the target's block margins, list indentation, monospace/white-space handling, heading size, or table layout, so promotion replaces the tail DOM and shifts everything below it — reintroducing P4, which the render layer exists to fix.

**Instead:** render the live tail through the *same* renderer, with the parser's best current interpretation, and re-render only that block. The cost is one block per frame, not one tree per frame — which was the actual goal.

### 5.5 Holdback rules — parser state, not a punctuation checklist

v1 listed `_`, `$`, `<`, `` ` ``, `[` as things to hold back. None of these are unambiguous openers — they are variously literal punctuation, currency, comparison operators, autolinks, HTML, emphasis, code, or math depending on dialect and context. The v1 rule was also self-contradictory: it said hold "until the terminator arrives," then said hold only the fence line, then added a 200 ms timeout that deliberately emits the broken markup the rule promised never to emit.

**Replacement:** holdback is a function of **parser state** — hold a reveal group when the parser reports an open inline construct whose rendering would change on close. Then state, explicitly:
- a **maximum holdback** (both time and atoms), and
- the **fallback rendering** when it expires (render as literal source text, and re-render when/if it closes).

A model can emit an unmatched `$` or a fence it never closes. There is no rule that both "never shows broken markup" and "never withholds output indefinitely" — the plan must pick a bound and name the trade, which v1 did not.

### 5.6 State-update discipline

- The reveal loop is the **sole writer** of visible state.
- `fullText` / the tape live in a **ref**; only the cursor is state. (v1's `parts[]`-in-state design re-rendered on every network chunk *and* every frame.)
- `memo()` the message so parent re-renders (timers, token counters, connection chips) don't cascade.
- With N concurrent streams: **one** scheduler, N cursors, **one** coalesced state write per frame. Fairness, teardown and frame-budget allocation across cursors must be specified — v1 said "one shared loop" without saying who gets the budget.

### 5.7 Scroll and layout

- **Stick-to-bottom with intent detection**: autoscroll only while within ~40 px of the bottom; any upward intent detaches until the user returns; show a "jump to latest" affordance while detached.
- Intent is not just wheel events: touch momentum, keyboard navigation, selection drag, focus-driven scroll, and resize all count. Enumerate them.
- One coalesced scroll adjustment per frame, on the reveal clock.
- **A block promotion or card insertion above the anchor changes height and moves the anchor** — the anchor must be content-relative, not pixel-absolute.
- Reserve height for known-size insertions (cards, images, charts).

---

## 6. Subsystem D — perceptual layer

### 6.1 The mechanism

Wrap newly-revealed groups and let CSS transition them in — `fadeIn` (opacity, ~250 ms) as the safe default; `blurIn` reads richer at some GPU cost. **Avoid anything that moves text** (slide, bounce): per-word layout shift, and it reads as a gimmick.

The reason this is high-value: a per-group fade **decouples perceived smoothness from reveal rate**. A 250 ms fade at 20 groups/sec can read smoother than un-faded atom-by-atom at 200/sec, for a fraction of the state churn.

### 6.2 Bounded animated window

Keep the animated window bounded — animate only the most recent groups and strip wrappers from older ones and from all completed messages, or a long conversation accumulates thousands of animated spans.

*Correction to v1:* v1 attributed a specific "last 30–60 atoms" window to FlowToken. FlowToken's README says only to disable animations on completed messages to lower memory footprint; it does not prescribe a moving window. The window is **our hypothesis**, and whether wrapper churn is actually cheaper than leaving wrappers in place is renderer-dependent and must be measured (§8), not asserted.

### 6.3 Lag composes — this is the trap

If the playout layer holds `L* = 250 ms` and the fade needs 250 ms to reach legibility, **fully-readable lag is ~500 ms**, not 250. v1 treated CSS as free, which means B and D could each pass their own targets and fail together.

**Therefore:** the latency budget is defined on **legible** lag, and `L*` and fade duration are drawn from one budget. Define the opacity threshold at which text counts as legible, plus behaviour for fade cancellation, overlapping fades, message completion mid-fade, and wrapper-removal timing — all of these change the metric as well as the correctness.

### 6.4 D depends on B

Per-group fades need **stable keys and wrapper lifetimes**. Those come from the atom/grouping model (§3.1), which B owns. Changing segmentation later remounts wrappers and replays or cancels fades. B and D share a contract; they cannot be specified independently (see §9.2).

---

## 7. Terminal semantics and edge cases

Under-specified rules are unimplementable rules. Each of these needs a decision before build.

### 7.1 Stream finished with backlog
Drain within `TAIL_DEADLINE`, or snap if over `SNAP_THRESHOLD` (§4.6). Never keep typing at leisure rate.

### 7.2 User pressed Stop
**Flush and snap immediately.** A UI that keeps typing after you told it to stop is the single worst outcome here. Cancel means the user has decided; only latency matters now.

### 7.3 Cancel / error races
"Snap to full" is not a specification without an ordering. Every turn carries an **`epoch`**; aborting transport, freezing the accepted prefix, cancelling scheduled reveals, and rejecting late events must be ordered, and **every scheduled callback checks its epoch before mutating**. Without this, a queued rAF callback or an in-flight network task can resurrect a cancelled message.

### 7.4 Tab hidden
rAF is **paused** in most browsers when hidden. On `hidden`: deschedule and keep buffering. On `visible`: snap the cursor to `end − (one L* window)` and resume pacing on the tail only. The user did not watch it and does not need it re-enacted.

### 7.5 History replay / reconnect / refresh
Persisted turns render **instantly and in full**. Only a live turn is paced. Requires an explicit `isLive` flag — which is exactly what a server-side implementation cannot provide.

### 7.6 Interleaved non-text events — barriers, not one global policy

v1 offered two global policies. **Both fail** on the same case: a blocking approval emitted after buffered explanatory text. Timeline-faithful ordering delays the approval behind the backlog while the model waits on the user — approaching deadlock or tool timeout. Priority pass-through shows the approval *before its own explanation*, then inserts text above it, shifting focus and scroll and making the user's action look causally premature.

**v2: classify events, don't pick one policy.**

| Class | Behaviour | Examples |
|---|---|---|
| **Inline** | Revealed at the cursor, in tape order | citations, inline cards, non-blocking status |
| **Barrier** | Forces drain-to-here (bounded by `TAIL_DEADLINE`), *then* reveals | approvals, blocking prompts, errors, turn end |
| **Overlay** | Renders immediately, out of band, not in the text flow | connection state, "running tool…" chips, token counters |

Additionally: a tool call is **not one event**. It has start, argument deltas, ready, approval, result, error and retry transitions, and its card updates in place while text continues. The tape (§3) must define which tape position owns the card, whether in-place updates bypass the cursor (they should — the card's *appearance* is barriered, its *updates* are not), and how a placeholder reserves height so the update doesn't shift layout.

### 7.7 Word groups vs. frame budget
A 35-atom URL or identifier may exceed one frame's budget. Rule: **defer the whole group** to the next frame (accumulating budget); never split a group, never overrun the rate. v1 left this undefined.

### 7.8 Accessibility
- `prefers-reduced-motion: reduce` → **disable pacing and all transitions**; render as it arrives. Non-negotiable.
- An `aria-live` region mutating 60×/sec is unusable — many readers restart from the beginning. Announce at **sentence boundaries** (not completion-only, which gives no progress on a long answer), keep the paced text out of the accessibility tree, and expose full text to AT.
- This needs a **screen-reader × browser validation matrix**, a focus-order test, and a live-region coalescing contract. `aria-hidden` paced text alongside a separately-mutating full-text node can announce future text or duplicate it. Untested a11y here is a real regression risk, not a checkbox.
- Never let pacing delay an actionable control (approve, stop) from becoming interactive.

### 7.9 DOM operations, not just painting
Per-group spans and live-tail re-rendering affect **selection, copy, find-in-page, link activation, code copy buttons, browser translation, text fragments, and screen-reader virtual buffers**. Selecting across a re-rendering tail can drop the selection. Find-in-page can fail to match text split across spans. Browser translation can fight per-word wrappers. This must be tested, and it is entirely absent from the source article.

### 7.10 Reveal vs. truth
Anything reading "the message" — copy, export, search, retry, token count, tool arguments — reads the **tape**, never the revealed prefix. Copying a half-typed message is a live bug class.

### 7.11 Memory and backpressure
Caps must exist: maximum message size, maximum tape length, compaction policy for long conversations, and defined overload behaviour. Hidden tabs, very long answers, and many concurrent streams otherwise grow `fullText` copies, segmentation arrays, tape entries and animated spans without bound.

### 7.12 Low-end devices
Time-derived stepping handles dropped frames automatically. Add a guard: if frames consistently exceed budget, degrade (coarser grouping, drop transitions). **Also specify the restore condition** — v1 said when to degrade but not when to recover, which produces a UI that degrades once and never comes back.

---

## 8. Measurement

Nothing ships without this. "It feels smoother" is how a latency regression ships.

### 8.1 Rig
Record real SSE transcripts **with arrival timestamps**, then replay deterministically. A live model cannot be A/B'd — the arrival pattern is not reproducible. Corpus must be stratified: short, long, code-heavy, table-heavy, CJK, emoji-heavy, tool-interleaved, slow-network, **and the worst observed bursts** — not just medians.

### 8.2 Metrics — v1's were largely unfalsifiable

| v1 metric | Why it failed | v2 replacement |
|---|---|---|
| Inter-paint interval σ | Trivially "improved" by one giant update, or by being uniformly slow; undefined for a single update | **Max visual stall (p95)** + **burst size distribution** + median cadence |
| Display lag | Never defined what "painted" means (React commit? next rAF? nonzero opacity? full opacity?) — a 250 ms fade moves the answer by hundreds of ms | **Insertion lag** and **legible lag** as two separate metrics, with a stated opacity threshold (§6.3) |
| Tail latency < 400 ms | Directly conflicts with a ≤400 ms fade unless an invisible first pixel counts as painted | Measured on **legible** lag; budget covers `L*` + fade together |
| Long tasks per message | Long Tasks API gives coarse browsing-context attribution, not per-message; "down" has no pass threshold; "zero under 500 chars" is unsupported | **Total blocking time** over the stream window, with a stated threshold |
| INP during stream | INP requires interactions; a passive stream has none | Prescribed interaction script (scroll, select, click stop) with sampling |
| Frames > 32 ms | An rAF-gap proxy, not compositor dropped frames; wrong on 120/144 Hz; flags a normal 30 Hz cadence as failure | Threshold **derived from measured refresh interval**, hidden periods excluded |

Add: **time to first readable clause** (tail latency does not catch delaying the first useful sentence) and **semantic throughput** (characters/sec is not comparable across English, CJK, code and emoji).

### 8.3 Correctness oracle
Recorded transcripts test timing, not correctness. Required:
- **Property tests over arbitrary chunk boundaries and Unicode** — the same logical stream, split every possible way, must produce identical output.
- **Golden markdown at every prefix** — no prefix renders as broken markup beyond the declared holdback bound.
- **Terminal invariant:** final DOM and final text are byte-identical to the un-smoothed render. This is the single most valuable test in the plan.
- Cancel/error race traces; non-text ordering traces.

### 8.4 Experiment design
Eight transcripts are fixtures, not a study. A real subjective comparison needs participant count, randomisation, counterbalancing, device/refresh-rate strata, language-reader strata, a reduced-motion cohort, confidence intervals, and a minimum effect size. It must be **factorial** — pacing × render × fade — because §6.3 shows the interaction is where the failure hides.

### 8.5 Telemetry constraints
Per-atom arrival/paint timestamps leak content length and generation behaviour. Specify sampling, aggregation, retention, an overhead budget, and an opt-out for sensitive conversations.

---

## 9. Phasing

### 9.1 Sequence

| Phase | Content | Gate |
|---|---|---|
| **P0** | Replay rig + baseline metrics (§8) + burst distribution from real traffic | Does a user-visible defect exist on representative hardware and content? If not, **stop here.** |
| **P1** | **Factorial evaluation**: baseline vs. fade-only vs. render-only vs. pacing-only, then combinations | Which subsystem earns its complexity? |
| **P2** | Content model (§3) — tape, units, decoding, epochs | Prerequisite for anything else surviving |
| **P3** | Whichever of C (render) / D (fade) / B (playout) P1 justified, in that order of expected value | Each against its own §8.2 metric |
| **P4** | Edge semantics (§7): visibility, replay, barriers, reduced-motion, DOM operations | Correctness — must precede general rollout |
| **P5** | Adaptive `L*` tuning, degradation recovery | Only if B was built |

**The evaluation moved before the build.** v1 put the "is pacing even necessary?" question last, as open-question 4, while simultaneously labelling pacing "the biggest single win." That ordering commits to the most complex and highest-risk subsystem before testing whether the cheapest one suffices.

### 9.2 Correction: the subsystems are *not* independent

v1 claimed the render and fade work could proceed in parallel with pacing. They cannot:

- **B ↔ D:** fades need stable atom keys and wrapper lifetimes; segmentation and grouping are B's. Change B and every wrapper remounts.
- **B ↔ C:** B sets update cadence and which prefixes are emitted; C decides which prefixes are *legal* and what each update costs. A controller tuned against plain text is mis-tuned after the live-tail strategy changes.
- **B + D:** latencies add (§6.3).

The only true parallelism is *after* the §3 atom contract is fixed. That contract is therefore P2, and it is the real prerequisite — not pacing.

---

## 10. Strategic assessment

**Is this worth building at all?** Conditionally, and the condition is P0. If recorded traces on representative hardware show no user-visible defect, the correct outcome is to stop — this is a category of feature that adds latency and is reverted after one fast-model or tool-heavy session.

**The strongest case is not for pacing.** A CSS fade adds *no intentional queueing latency*, needs no feedback controller, and softens the small frequent chunks that dominate normal streams. If the transport already emits often enough, the playout layer is machinery solving a problem users barely perceive, while the fade captures most of the perceived polish for a fraction of the risk.

**But fade-only has hard limits, and they should not be glossed:** a fade does not *serialise* a 200-atom burst — it makes 200 atoms bloom simultaneously. It does not fill a 400 ms silence, does not fix full-tree re-parsing, does not solve event ordering, and does not reduce layout churn. Blur may add compositor cost.

**Ranking by expected value per unit risk:**
1. **C (render layer)** — likely justifiable on its own from measured blocking time on markdown-heavy answers, independent of any perceptual argument. Lowest risk, clearest metric.
2. **D (fade)** — highest perceived polish per unit effort; no added latency; needs only the atom contract.
3. **B (playout)** — build only if P0's burst distribution proves C and D leave a material problem. If it is needed, start with a **fixed, small `L*` jitter buffer** and an explicit latency budget; add producer tracking and adaptive `L*` only if a fixed buffer measurably fails.

The largest blast radius here is correctness and accessibility, not performance. Pacing touches transport, Unicode, markdown parsing, animation, scroll, selection, and screen readers simultaneously — which is why §8.3's terminal invariant (final output identical to un-smoothed) is the test that matters most.

---

## 11. Anti-goals

- **Not building a typewriter aesthetic.** The goal is *invisible* pacing; the ideal outcome is that no one notices smoothing exists. A visible typewriter on a fast model is a downgrade.
- Not smoothing on the server.
- Not animating completed or historical messages.
- Not shipping tunables as user-facing settings. Fix the defaults with data.
- Not asserting a rate envelope, a window size, or a constant that the rig has not measured.

---

## 12. Open decisions

1. **Markdown dialect and enabled extensions** (§5.2) — determines block stability and therefore the whole render design. Blocking for C.
2. **Link-reference definitions**: disable, or accept full re-render on resolution? Blocking for C.
3. **Holdback bound** (§5.5): the maximum time/atoms to withhold unclosed markup, and the fallback rendering. Blocking for C.
4. **Legible-lag budget** (§6.3): the single number `L*` + fade duration must fit inside.
5. **Barrier classification** (§7.6): which events are barriers is a product call, not a technical one.
6. **Reveal grouping**: atom or word, per locale — settle with the §8.4 factorial, not by preference.

---

## Changelog

**v2 (2026-08-07)** — full revision in response to `smooth-streaming-plan.review.md`.

Material corrections to v1:
- **Control law replaced.** v1's `p̂ × 1.10 + backlog/T` has no positive equilibrium; it drives the buffer to empty, which is the un-smoothed state. Replaced with an adaptive playout/jitter buffer with a positive latency setpoint (§4.2–4.4).
- **Constants derived, not chosen.** v1's `TARGET_DRAIN_SECONDS` / `CPS_MAX` / `HARD_LAG_CEILING` were mutually unsatisfiable; `SNAP_THRESHOLD` is now derived (§4.6).
- **Units unified.** v1 mixed graphemes, UTF-16 code units, words and chars/sec in one loop (§3.1).
- **EMAs given wall-clock time constants.** v1's per-frame α was frame-derived — the same error it criticised in the article (§4.4).
- **Flat `fullText` replaced with a typed event tape** (§3); added transport decoding boundaries (§3.2) and non-append updates (§3.3).
- **Provisional-tail rule added**; `Intl.Segmenter` alone does not make a *growing* buffer safe (§4.8).
- **Markdown block stability retracted.** Setext headings, reference definitions, tables, list tightness and fences all reinterpret earlier source; "committed forever" replaced with a parser-supplied commit horizon (§5.1–5.2). Fence-parity heuristic dropped (§5.3). Plain-text live tail dropped — it traded parse jank for layout jank (§5.4). The punctuation-checklist holdback replaced with parser state plus an explicit bound and fallback (§5.5).
- **Single non-text ordering policy replaced with event classes** — both v1 policies failed on a blocking approval behind buffered text (§7.6).
- **Epoch/generation identity added** for cancel races (§7.3).
- **Metrics rewritten** — most v1 targets were gameable, undefined, or unattributable (§8.2). Added correctness oracle (§8.3) and a real experiment design (§8.4).
- **Phasing reordered**: evaluation before build; the atom contract, not pacing, is the prerequisite. v1's claim that the subsystems were independent is retracted (§9.2).
- **Added:** memory/backpressure caps, DOM-operation effects (selection, copy, find-in-page, translation), a11y validation matrix, degradation *recovery*, telemetry privacy.

Citation corrections:
- The "last 30–60 atoms" animated window was **our hypothesis**, not FlowToken's guidance (§6.2).
- Server-side smoothing does **not** compound per passive proxy hop (§4.10).
- Background rAF is **paused**, not throttled to a predictable ~1 Hz (§2.2).
- "~200 chars/sec is unachievable" scoped to "uncontrolled and hardware-dependent" (§2.2).
- Korean and Vietnamese *do* use spaces; the failure mode differs from Japanese/Chinese/Thai (§4.9).
- Streaming-rate envelope demoted from asserted fact to a figure the rig must measure (§2.3).

---

## Sources

- [Upstash — *Smooth Text Streaming in AI SDK v5*](https://upstash.com/blog/smooth-streaming) (primary)
- [AI SDK — `smoothStream` reference](https://ai-sdk.dev/docs/reference/ai-sdk-core/smooth-stream)
- [FlowToken](https://github.com/Ephibbs/flowtoken)
- [MDN — `requestAnimationFrame`](https://developer.mozilla.org/en-US/docs/Web/API/Window/requestAnimationFrame)
- [MDN — `Intl.Segmenter`](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Intl/Segmenter)
- [Unicode UAX #29 — Text Segmentation](https://www.unicode.org/reports/tr29/)
- [CommonMark 0.31.2 — Fenced code blocks](https://spec.commonmark.org/0.31.2/#fenced-code-blocks)
