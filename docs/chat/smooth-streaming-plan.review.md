# Adversarial Review: Smooth Text Streaming Concepts & Plan

> **Scope note.** This review was written against **v1** of `smooth-streaming-plan.md`.
> `smooth-streaming-plan.md` in this directory is **v2**, revised in response to it — see that
> document's Changelog for what each finding changed. Section numbers cited below refer to v1
> and will not line up with v2. The review is retained unedited as the audit trail for why v2
> looks the way it does.
>
> Reviewer: Codex (gpt-5.6-sol), adversarial mode. Constraint given: concepts only, no repository
> access, no code.

**Verdict**

This is not ready to drive implementation. Its central controller is dimensionally inconsistent, mathematically unable to meet its own lag deadlines, and designed to eliminate the backlog that is supposed to absorb jitter. The render plan assumes Markdown blocks become permanently stable when many Markdown constructs are explicitly nonlocal, and the event-ordering section offers two policies that both fail on blocking tool interactions. The measurement plan then hides these defects behind metrics that are undefined, easy to game, refresh-rate-dependent, or not attributable to a message. Build from this as written and the likely result is extra latency, semantic flicker, and a feature flag that gets turned off.

**Confirmed correct**

- The Upstash hook really does increment its cursor by exactly one UTF-16 code unit on each `requestAnimationFrame` callback that passes `time - lastTime > 5`; it does not accumulate elapsed time or reveal multiple units. On an ordinary 60 Hz display, where callbacks are about 16.7 ms apart, that is about 60 units/s, not 200. At 120 Hz and 144 Hz, it is about 120 and 144 units/s. This follows directly from the published hook and MDN's statement that rAF frequency generally matches display refresh rate ([Upstash article](https://upstash.com/blog/smooth-streaming), [MDN rAF](https://developer.mozilla.org/en-US/docs/Web/API/Window/requestAnimationFrame)).

- The 4,000-character arithmetic is correct for the chosen example: `4000 / 60 = 66.7` seconds. If those characters arrive in 15 seconds while 900 are revealed, 3,100 remain and require another 51.7 seconds at 60 units/s. What is not established is that 15 seconds is representative.

- JavaScript string indices are UTF-16 code-unit indices. A raw `slice(0, i)` can expose a lone surrogate and can split a multi-code-point grapheme such as an emoji modifier, flag, ZWJ sequence, or base-plus-combining-mark sequence. MDN documents all of these distinctions ([MDN String](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/String#utf-16_characters_unicode_code_points_and_grapheme_clusters)).

- `Intl.Segmenter` with grapheme granularity is the right standard primitive for extended grapheme boundaries in a complete string. Unicode calls grapheme clusters "user-perceived characters," and MDN describes `Intl.Segmenter` as locale-sensitive segmentation into graphemes, words, or sentences ([Unicode UAX #29](https://www.unicode.org/reports/tr29/), [MDN Intl.Segmenter](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Intl/Segmenter)). This does not validate the plan's streaming use of it; see below.

- Current AI SDK documentation and source set `delayInMs` to 10 ms and `chunking` to `word` by default. The implementation smooths `text-delta` and `reasoning-delta`; for any other part it first flushes buffered text and then enqueues the non-text part without awaiting the delay. The docs explicitly call out poor default word chunking for Chinese, Japanese, Korean, Vietnamese, and Thai, and recommend `Intl.Segmenter` ([AI SDK reference](https://ai-sdk.dev/docs/reference/ai-sdk-core/smooth-stream), [AI SDK source](https://github.com/vercel/ai/blob/main/packages/ai/src/generate-text/smooth-stream.ts)).

**Wrong or unsupported**

1. **Claim: the constants form a coherent starting configuration.** They do not. `TARGET_DRAIN_SECONDS=0.35` and `CPS_MAX=400` can drain at most 140 characters within the target. `HARD_LAG_CEILING=1500` permits a backlog whose minimum drain time at the cap is 3.75 seconds. The terminal rule's 400 ms bound permits at most 160 characters at the same cap. For any remainder from 161 through 1,500, either the cap, the drain target, or the terminal bound must be violated. It should instead state one invariant and derive the thresholds from it, for example making the snap threshold no greater than `maximum_rate * maximum_tail_time`, with every quantity expressed in the same reveal unit.

2. **Claim: the recommended law holds display lag near a target.** It does not have a positive lag setpoint. Ignoring clamping and smoothing, with producer rate `p` and backlog `b`, the law asks for `1.1p + b/T`; backlog therefore changes at `p - (1.1p + b/T) = -0.1p - b/T`. Its only feasible equilibrium is the boundary `b=0`, where the consumer starves. At that boundary each new atom is exposed as soon as it arrives, so arrival jitter returns. It should say either that the goal is fastest possible catch-up, or define a real playout-delay/backlog target and a controller around its error. It cannot claim both.

3. **Claim: the controller's units are characters.** The state table calls `revealedIndex` a grapheme index, while backlog is `fullText.length - revealedIndex`; `fullText.length` is UTF-16 code units. The recommendation then switches the reveal atom to words while retaining CPS, fractional character carry, and character thresholds. Subtracting graphemes from code units is meaningless, and advancing one word against a character-rate budget produces rate swings proportional to word length. It should define one canonical position unit, a conversion policy for variable-length atoms, and separate byte/code-unit offsets used only for slicing from the perceptual work units used for pacing.

4. **Claim: output-rate EMA smoothing is refresh-rate-independent.** `ALPHA=0.15 per frame` is explicitly frame-derived. Its response is roughly twice as fast at 120 Hz as at 60 Hz and changes again under dropped frames. That contradicts the argument for time-derived pacing. It should specify the filter by a wall-clock time constant and derive the per-update coefficient from elapsed time.

5. **Claim: the article's approximately 200 chars/s is unachievable.** This is too absolute. The article is wrong on common 60/120/144 Hz displays, but a callback cadence just over 5 ms can approach 200 increments/s. At exactly 200 Hz the strict `>` gate would usually advance every second callback, while at about 196 Hz it could advance every callback. At refresh rates above 200 Hz it advances on only some frames. It should say that the rate is quantized by callback cadence, is about 60/120/144 on those common displays, and is not controlled to 200; not that approximately 200 is physically impossible.

6. **Claim: background rAF is throttled to approximately 1 Hz.** MDN says rAF is *paused* in most browsers in background tabs or hidden iframes. Timer throttling and rAF scheduling are not interchangeable, and behavior is browser/power-policy dependent. It should say "usually paused, otherwise aggressively throttled; never rely on a cadence" ([MDN rAF](https://developer.mozilla.org/en-US/docs/Web/API/Window/requestAnimationFrame)).

7. **Claim: the `typewriterSpeed` constant is inert for any value below 16.7 ms.** Only on an idealized 60 Hz schedule. It is not inert on 120/144 Hz displays, around threshold equality, or under jitter. It should scope the statement to a stable 60 Hz callback cadence and note the strict comparison.

8. **Claim: modern Sonnet/GPT-class streaming sustains 100-400 chars/s.** This is a plausible envelope, not a fact established by the document. Rate varies by exact model, provider, service tier/load, reasoning mode, output language, tokenizer, and proxy coalescing. Artificial Analysis currently reports GPT-5 provider medians around 78.5-88.6 output tokens/s, which might map into the claimed character range for typical English but not by a fixed conversion ([Artificial Analysis GPT-5 provider benchmark](https://artificialanalysis.ai/models/gpt-5/providers)). The lower and upper bounds are not justified as sustained percentiles, and character/token ratios differ sharply by language. It should require measured post-transport character or grapheme arrival rates from the replay corpus, reporting median and tail distributions by model, provider, and language. The 67-second calculation may remain as a conditional example.

9. **Claim: `Intl.Segmenter` by itself fixes streaming grapheme breakage.** It fixes segmentation of the string currently supplied, but the final segment at the end of a growing buffer is provisional. A later chunk can append a combining mark, variation selector, skin-tone modifier, regional indicator, or ZWJ continuation to the last already-revealed segment. Unicode also warns that adjacent characters alone can be insufficient to determine a boundary. The plan needs an end-of-buffer holdback rule for the last provisional grapheme/word and a terminal flush, not just a segmenter ([Unicode UAX #29](https://www.unicode.org/reports/tr29/)).

10. **Claim: whitespace chunking breaks all listed languages for the same reason.** The attribution to AI SDK documentation is correct, but the explanation is not. Chinese, Japanese, and Thai commonly lack inter-word spaces; standard Korean uses spacing, and Vietnamese visibly uses spaces between syllables even when lexical words may contain multiple syllables. The default AI SDK regex may produce linguistically bad Korean/Vietnamese chunks, but it does not necessarily buffer the entire sentence as one giant chunk. It should distinguish "no useful delimiter" from "delimiter is not the desired lexical boundary." MDN's concrete no-whitespace list is Japanese, Chinese, Thai, Lao, Khmer, Myanmar, and similar scripts, not the plan's blanket CJK/Thai/Vietnamese explanation ([MDN Intl.Segmenter](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Intl/Segmenter#basic_usage_and_difference_from_string.prototype.split)).

11. **Claim: a monotonic pure prefix means "no re-flow of already-painted text, ever."** False. Appending to an inline run can move the current word to the next line. More importantly, later Markdown can reinterpret earlier source: a following `===` can turn a paragraph into a setext heading, later blank lines can change list tightness, a link-reference definition can resolve an earlier reference, and a table delimiter can turn a preceding line into a header. Prefix monotonicity prevents deletion at the text model level; it does not freeze layout or parse semantics. It should claim only append-only source identity.

12. **Claim: counting fence markers and checking parity identifies a closed code block.** CommonMark fences have a marker character, opening length, indentation rules, and a closing fence must use the same marker and at least the opening length. Backticks inside content do not close a fence merely because a count becomes even. It should defer to the selected Markdown parser's block state, not marker parity ([CommonMark 0.31.2, fenced code blocks](https://spec.commonmark.org/0.31.2/#fenced-code-blocks)).

13. **Claim: the Markdown holdback rules are implementable as listed.** `_`, `$`, `<`, and backticks are not unambiguous openers. They can be literal punctuation, currency, comparisons, autolinks, HTML, emphasis, code, or math depending on dialect and context. Holding an opening code fence "until the terminator arrives" would withhold the entire code block, while the next sentence says to hold only the fence line. A 200 ms timeout then deliberately emits the broken syntax that the rule claims never to emit. It should define a Markdown dialect, parser state, maximum holdback, and exact fallback semantics; a punctuation checklist is not a specification.

14. **Claim: server-side smoothing becomes strictly worse when a proxy hop exists because delay compounds.** A single smoothing transform delays once regardless of how many passive hops carry it. Delay compounds only if multiple hops smooth independently or backpressure changes upstream behavior. Server smoothing has real disadvantages here, but it can also reduce client update frequency and centralize behavior. It should make a product tradeoff, not claim strict dominance from hop count.

15. **Claim: FlowToken states the bounded last-30-to-60-atoms caveat.** The checked FlowToken README says to disable animations on completed messages to lower memory footprint. It does not prescribe a 30-60 atom moving window or stripping old wrappers during a live message. That window may be a reasonable hypothesis, but the attribution is false ([FlowToken README](https://github.com/Ephibbs/flowtoken/blob/main/README.md)).

16. **Claim: the section 7 objective targets are falsifiable as written.** Several are not:

   - `Inter-paint interval sigma` is trivially improved by one giant update or uniformly slow updates. With one update it is undefined or vacuously zero. It needs update size, stall duration, and mean/percentile cadence beside variance.
   - `Display lag` never defines whether "painted" means React commit, the next rAF, first nonzero opacity, or fully opaque. Arbitrary DOM paint timestamps are not directly exposed as a per-character browser metric, and a 400 ms fade changes the answer by hundreds of milliseconds.
   - `Tail latency <400 ms` conflicts with a fade lasting up to 400 ms unless the metric starts counting an effectively invisible first pixel as painted.
   - `Long tasks per message` cannot generally be attributed to one message by the Long Tasks API; it reports main-thread tasks of at least 50 ms and coarse browsing-context attribution. "Down" has no pass threshold, while zero below 500 characters is an unsupported absolute ([MDN PerformanceLongTaskTiming](https://developer.mozilla.org/en-US/docs/Web/API/PerformanceLongTaskTiming)).
   - `INP during stream` does not exist without defined user interactions. INP is based on the page's longest interaction latency after excluding some outliers, not a passive streaming interval. A replay must prescribe representative interactions and sampling ([MDN PerformanceEventTiming](https://developer.mozilla.org/en-US/docs/Web/API/PerformanceEventTiming)).
   - `Frames >32 ms` is only an rAF-gap proxy, not a compositor dropped-frame count. It misses dropped frames on 120/144 Hz displays and treats an ordinary 30 Hz cadence as failure. The threshold must derive from the measured refresh interval and exclude hidden periods.

**Design holes**

1. **Slow producer with backlog near zero -> smoothing disappears.** At 5-20 cps, the buffer repeatedly empties. `CPS_MIN=30` cannot reveal data that does not exist, so every newly completed word is exposed at arrival cadence. The advertised decoupling has no playout buffer, startup delay, or target backlog to absorb jitter.

2. **Bursty producer -> sawtooth despite the rate EMA.** A burst raises backlog and target rate; the overdriven controller empties it; rate remains elevated after starvation; the next burst repeats the cycle. Output EMA softens the slope but does not create a stable operating point. It can also violate the 400 ms terminal bound while it slowly ramps upward.

3. **Eight-second tool call -> undefined controller restart.** Once preceding text drains, the consumer stalls at zero. If producer-rate EMA only updates on arrivals, it remains stale for eight seconds; if it decays on time, the plan gives no decay rule. On resumed text, the stale-high version can dump the first burst and the stale-low version can crawl before catching up. Neither behavior is specified.

4. **A stream resumes with a continuation of the final grapheme -> already-painted content changes.** The last visible atom may acquire a modifier or combining character. A monotonic cursor cannot retract it, yet Unicode correctness requires treating the extended sequence as one atom. The plan has no provisional-tail state.

5. **Word-level atoms meet character-level budget -> stall or overshoot.** A 35-character URL, identifier, or CJK segment may cost more budget than one frame has. The plan does not say whether to reveal it early, wait until enough fractional character credit accrues, split it despite the word policy, or exceed the deadline.

6. **Unmatched syntax meets holdback -> indefinite content suppression or deliberate flicker.** A model can emit a long fenced block, an unmatched `$`, a literal underscore, or malformed Markdown. Waiting for a closer can hide minutes of useful output; the 200 ms escape emits unstable markup. There is no answer that satisfies the stated "never reveal" rule.

7. **Setext heading -> previously committed paragraph is not stable.** A line that looked complete becomes a heading when the next line is `===`. The proposed content-hash-by-block-index cache either renders stale semantics or reparses a supposedly immutable block.

8. **Reference definition -> arbitrarily old blocks change.** A definition near the end can resolve `[label][id]` near the start. Footnote definitions and some extensions have the same nonlocal effect. "Never re-render committed blocks" is incompatible with the language being rendered.

9. **Table -> block promotion cannot be line-local.** The first line is a paragraph until the delimiter row arrives; every later row belongs to one table container. Committing rows independently cannot preserve a single table AST or DOM. Waiting until the table ends makes the whole table the live tail.

10. **Nested/loose list -> blank lines do not terminate a stable block.** A list item may contain multiple paragraphs, nested lists, block quotes, and fenced blocks. Later blank lines can switch the entire list from tight to loose. "Completed list item" has no definition without incremental parser state and lookahead.

11. **Fence with blank lines -> paragraph splitting corrupts structure.** Blank lines inside a fence are content, not block boundaries. HTML blocks, block quotes with lazy continuation, indented code, and multi-paragraph list items create similar counterexamples. The committed/live-tail split is not a string splitting problem.

12. **Plain-text live tail -> promotion causes the layout jump P4 was supposed to prevent.** A raw span does not have Markdown block margins, list indentation, code font/white-space, link styling, heading size, or table layout. Promotion can replace the entire tail DOM and move surrounding content. The render optimization trades parse jank for semantic and layout jank.

13. **P1 and P3 built "independently" -> incompatible atom identity.** P3 needs stable per-word keys and wrapper lifetime; P1 chooses segmentation, locale, provisional-tail behavior, and reveal grouping. Changing P1 later remounts wrappers and replays or cancels fades. They share the atom model and cannot be specified independently.

14. **P1 and P2 built "independently" -> different boundaries and performance conclusions.** P1 determines update cadence and visible prefixes; P2 determines which prefixes are legal/stable and how costly each update is. A controller tuned against plain text or full reparsing will be wrong after the live-tail strategy changes.

15. **Pacing plus fade -> accumulated perceptual lag.** If pacing allows 350 ms arrival-to-insertion and fade needs another 250-400 ms to become readable, fully legible lag is 600-750 ms. The controller and metrics treat CSS as free, so separate phases can each pass and fail when combined.

16. **Blocking approval after buffered explanatory text -> both non-text policies fail.** Timeline-faithful ordering delays the approval behind backlog while the model waits for user action; with a large backlog or syntax holdback this can approach a deadlock or tool timeout. Forced drain causes the snap the pacing layer exists to avoid. Priority pass-through displays the approval before its explanation, then inserts text above it, shifts focus/scroll, and may make the user's action look causally premature. The plan needs event classes and barrier semantics, not one global policy.

17. **Streaming tool input/result -> "arrival position" is not one event.** Tool calls have start, argument deltas, ready/approval, result, error, and retry transitions; citations may anchor ranges not yet revealed; a card can update in place while text continues. The plan never defines which position owns the card, whether updates bypass the cursor, or how placeholders reserve height.

18. **Cancel/error races -> late chunks can resurrect a completed turn.** "Snap to full" does not define the atomic order among aborting transport, freezing the accepted prefix, cancelling scheduled reveals, and rejecting late events. Without a generation/epoch identity, a queued callback or late network task can mutate the cancelled message.

19. **Shared scheduler plus "one state write per frame" -> no ownership rule.** With N concurrent messages, does the single scheduler make N React writes, one aggregate write, or update only one cursor per frame? Fairness, teardown, starvation, and frame-budget allocation are unspecified.

20. **Detached scrolling plus late block promotion/tool insertion -> anchor still moves.** A fixed 40 px threshold does not define wheel/touch momentum, keyboard navigation, selection drag, focus-driven scroll, resize, virtualized history insertion, or what happens when a promoted block changes height above the anchor.

21. **SPECULATIVE: bounded wrappers may not reduce compositor cost as assumed.** Removing wrapper/animation state from old atoms can itself cause frequent reconciliation and style invalidation. Whether 30, 60, or any moving window is cheaper is renderer- and browser-dependent and requires measurement; it cannot be asserted as a universal bound.

**Missing entirely**

- **A canonical content model.** A flat `fullText` cannot represent interleaved text parts, reasoning, tool lifecycle events, citations anchored to spans, cards, approvals, files, or replacements. The plan needs an append-only typed event tape with stable IDs and explicit text offsets before it can discuss ordering.

- **Transport decoding boundaries.** UTF-8 bytes, SSE records, JSON deltas, and JavaScript strings have different boundaries. There is no rule ensuring streaming UTF-8 decoding before segmentation, handling a provider chunk that divides a sequence, or preventing duplicated/lost deltas on reconnect.

- **Non-append updates.** Providers and client libraries can replace snapshots, reconcile tool parts, redact reasoning, retry a step, or switch branches. Prefix-only input is an assumption, not an invariant, and there is no fallback when it fails.

- **A Unicode streaming contract.** Locale selection, mixed-script text, `Intl.Segmenter` availability/fallback, end-of-buffer provisional segments, normalization, and consistency across browser versions are absent. "Use Segmenter" is not the contract.

- **A selected Markdown dialect and incremental-parse model.** CommonMark versus GFM tables, math, footnotes, raw HTML, custom components, and sanitizer behavior materially change block stability. There is no definition of a commit boundary or invalidation range supplied by the actual parser.

- **Memory and backpressure limits.** Hidden tabs, huge responses, many concurrent streams, `fullText` copies, segmentation arrays, event tapes, and animated spans can grow without bound. There is no maximum message size, compaction policy, or overload behavior.

- **First-readable-content and semantic-throughput metrics.** Tail latency does not catch delaying the first useful sentence, and character rate is not comparable across English, CJK, code, and emoji. Time to first readable clause, maximum visual stall, burst size, and words/information units per second are absent.

- **A definition of visibility under CSS animation.** Opacity threshold, fade cancellation, overlapping fades, message completion during an active fade, and wrapper removal timing determine both correctness and every lag metric.

- **DOM behavior outside painting.** Per-word spans and live-tail replacement affect selection, copy, find-in-page, link activation, code copy buttons, browser translation, text fragments, and screen-reader virtual buffers. The document considers memory but not user operations on the DOM.

- **Accessibility validation with actual assistive technology.** `aria-hidden` paced text plus a separately mutating full-text node can announce future text or duplicate content; completion-only announcements provide no progress during long answers. There is no screen-reader/browser matrix, focus-order test, or live-region coalescing contract.

- **Controller overload and recovery semantics.** There is no definition for EMA sampling, initialization, idle decay, rate-change limits, hysteresis, saturation telemetry, tab-resume reset, or degradation recovery. The low-end guard says when to degrade but not when to restore.

- **A correctness test oracle.** Recorded transcripts cover arrival timing, not invariants. Missing are property tests over arbitrary chunk boundaries and Unicode, deterministic visibility transitions, cancel/error races, Markdown golden outputs at every prefix, non-text ordering traces, and assertions that final DOM/text equals the unsmoothed result.

- **A real experiment design.** Eight transcripts are fixtures, not a subjective study. There is no participant count, randomization, counterbalancing, device/refresh-rate strata, language-reader strata, reduced-motion cohort, confidence interval, or minimum effect size. There is also no factorial test separating pacing, render optimization, and fade interactions.

- **Telemetry constraints.** Per-character arrival/paint timestamps can expose generated content length and behavior. Sampling, aggregation, privacy, retention, overhead budgets, and disabling instrumentation for sensitive chats are not addressed.

**Strategic take**

Open question 4 should be the first product gate after the replay rig, not an item left at the end while phase P1 is labeled the "biggest single win." A CSS fade has a strong strategic advantage: it adds no intentional queueing latency, needs no unstable feedback controller, and can soften the edges of the small frequent chunks that dominate many normal streams. If the existing transport already emits often enough, P1 is machinery solving a problem users barely see, while P3 may capture the perceived polish.

That case has limits. Fade alone does not serialize a 200-character burst; it makes 200 characters bloom simultaneously. It does not fill a 400 ms network silence, fix full-tree Markdown parsing, solve event ordering, or reduce layout churn. Blur may add compositor cost, and per-word wrappers require the same stable atom identity the plan has not designed. Therefore the right early comparison is not "P3 magically solves everything" but baseline versus fade-only versus render-only versus pacing-only, followed by the combinations. Measure the worst burst traces as well as median traces.

My call: run P0, then test the smallest fade-only treatment and the render-cost treatment before authorizing P1. If fade-only wins most subjective preference without hurting first-readable time, delete P5 and require hard evidence before building any controller. If large bursts remain visibly bad, use a bounded jitter buffer with a simple explicit latency budget before attempting producer tracking. The current P1 control law has no stable lag target and is not a refinement-ready foundation.

As a whole, this effort is only worth doing if P0 demonstrates a user-visible defect on representative hardware and content. P2 may be justified independently by measured long tasks on Markdown-heavy responses. P1/P5 are polish with a large correctness and accessibility blast radius; absent measured arrival burst distributions and user preference, they are exactly the kind of latency-adding feature that will be reverted after one fast-model or tool-heavy session.

**If I could change only three things**

1. Replace the flat string/cursor/controller specification with a typed, append-only event timeline and one mathematically consistent latency contract. Define canonical offsets, perceptual work units, provisional Unicode tail, event barriers, terminal overrides, and derive the maximum backlog/snap threshold from the maximum rate and tail deadline.

2. Delete the paragraph/list-item committed/live-tail rules and replace them with a parser-aware invalidation contract for one named Markdown dialect. No block is "committed forever" unless the parser can prove it; define how nonlocal references, tables, lists, fences, HTML, math, and plain-text-to-rendered promotion behave.

3. Reorder the phases to P0, then a factorial P3-only/P2-only/P1-only evaluation with measurable paint definitions and representative interactions. Ship P2 if it removes measured jank, ship P3 if it wins preference without latency, and do not build P1/P5 unless recorded burst distributions prove that CSS and render work leave a material problem.
