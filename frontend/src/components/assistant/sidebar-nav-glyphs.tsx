import { useCallback, useEffect, useId, useRef, useState, type RefObject } from "react";

/* Easter-egg redraws of the lucide House, CalendarClock, Monitor, LayoutGrid
   and ShieldCheck icons for the assistant sidebar. At rest each is
   geometrically identical to the lucide original; hovering the row it sits
   in plays a small animation twice, and leaving early lets the current play
   finish at the rest frame instead of snapping. Reduced motion never starts
   one. */

interface GlyphProps {
  readonly className?: string;
}

/** Plays per hover: enough to be noticed, not enough to nag. */
const PLAYS_PER_HOVER = 2;

function prefersReducedMotion(): boolean {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** Tracks hover on the row (the nearest link or button) around `ref`.
 *  `hovering` follows the pointer exactly. `playing` turns on at pointer
 *  enter (never under reduced motion) and turns off at a cycle boundary,
 *  reported through `endCycle()`, once the pointer has left or the hover has
 *  had its plays. */
function useRowHoverPlayback(ref: RefObject<SVGSVGElement | null>) {
  const hovered = useRef(false);
  const plays = useRef(0);
  const [hovering, setHovering] = useState(false);
  const [playing, setPlaying] = useState(false);

  useEffect(() => {
    const row = ref.current?.closest("a, button");
    if (!row) return;
    const enter = () => {
      hovered.current = true;
      plays.current = 0;
      setHovering(true);
      if (!prefersReducedMotion()) setPlaying(true);
    };
    const leave = () => {
      hovered.current = false;
      setHovering(false);
    };
    row.addEventListener("pointerenter", enter);
    row.addEventListener("pointerleave", leave);
    return () => {
      row.removeEventListener("pointerenter", enter);
      row.removeEventListener("pointerleave", leave);
    };
  }, [ref]);

  /** Call at the end of each cycle; returns whether playback stopped. */
  const endCycle = useCallback(() => {
    plays.current += 1;
    const done = !hovered.current || plays.current >= PLAYS_PER_HOVER;
    if (done) setPlaying(false);
    return done;
  }, []);

  /** Stops playback if the pointer has gone, whatever the cycle count. */
  const stopIfLeft = useCallback(() => {
    if (!hovered.current) setPlaying(false);
  }, []);

  return { hovering, playing, endCycle, stopIfLeft };
}

/** Calls `draw(elapsedInCycle)` every frame while playing, ending each cycle
 *  through `endCycle()`, so playback always stops on the rest frame. For
 *  animations CSS cannot express portably. */
function useHoverFrameLoop(
  ref: RefObject<SVGSVGElement | null>,
  cycleMs: number,
  draw: (elapsedInCycle: number) => void,
) {
  const { playing, endCycle } = useRowHoverPlayback(ref);
  const drawRef = useRef(draw);
  drawRef.current = draw;

  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    let start: number | undefined;
    let cycle = 0;
    const tick = (now: number) => {
      start ??= now;
      const elapsed = now - start;
      if (Math.floor(elapsed / cycleMs) > cycle) {
        cycle += 1;
        if (endCycle()) {
          drawRef.current(0);
          return;
        }
      }
      drawRef.current(elapsed % cycleMs);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [playing, endCycle, cycleMs]);

  return playing;
}

function easeInOutCubic(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2;
}

const SVG_ATTRS = {
  "aria-hidden": true,
  focusable: false,
  width: 24,
  height: 24,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 2,
  strokeLinecap: "round",
  strokeLinejoin: "round",
} as const;

/* A chimney pops up out of the right roof slope with a small overshoot,
   then puffs of smoke rise from it, drifting right and swelling as they
   fade. The chimney is clipped to the area above the roof line (the clip
   sits on an untransformed group, so it stays put while the chimney moves
   inside it), which lets it rise out of the roof and sink back into it.
   The chimney top is near the top of the viewBox, so the svg lets the smoke
   overflow into the row's padding. Each puff rises twice (CSS iteration
   count). On leave the chimney sinks and any puffs in flight fade out; the
   puffs unmount once that fade has finished. */
const SMOKE_PUFFS = [0, 0.7, 1.4];

/** House whose chimney pops up and smokes while the row is hovered. */
export function HomeGlyph({ className }: GlyphProps) {
  const ref = useRef<SVGSVGElement>(null);
  const clipId = `nav-glyph-roof-${useId().replace(/[^\w-]/g, "")}`;
  const { hovering, playing, stopIfLeft } = useRowHoverPlayback(ref);
  return (
    <svg
      ref={ref}
      {...SVG_ATTRS}
      overflow="visible"
      className={className}
      data-hovering={hovering || undefined}
    >
      <clipPath id={clipId}>
        {/* Everything above the centre line of the right roof slope. */}
        <path d="M13 2.22L22 9.93V-12H13Z" />
      </clipPath>
      <g clipPath={`url(#${clipId})`}>
        <path className="nav-glyph-chimney" d="M16 4.8V2.5h2.5v4.4" />
      </g>
      <g className="nav-glyph-smoke" onTransitionEnd={stopIfLeft}>
        {playing
          ? SMOKE_PUFFS.map((delay) => (
              <circle
                key={delay}
                className="nav-glyph-smoke-puff"
                cx="17.25"
                cy="0.5"
                r="1.4"
                fill="currentColor"
                stroke="none"
                style={{ animationDelay: `${0.45 + delay}s` }}
              />
            ))
          : null}
      </g>
      <path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8" />
      <path d="M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    </svg>
  );
}

/** Calendar whose clock hands spin, the minute hand twice as fast. */
export function AutomationsGlyph({ className }: GlyphProps) {
  const ref = useRef<SVGSVGElement>(null);
  const { playing, endCycle } = useRowHoverPlayback(ref);
  return (
    <svg ref={ref} {...SVG_ATTRS} className={className} data-playing={playing || undefined}>
      <path d="M16 2v4" />
      <path d="M21 7.5V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h3.5" />
      <path d="M3 10h5" />
      <path d="M8 2v4" />
      <circle cx="16" cy="16" r="6" />
      <path className="nav-glyph-minute-hand" d="M16 16.2V14" onAnimationIteration={endCycle} />
      <path className="nav-glyph-hour-hand" d="M16 16.2l1.6 1" />
    </svg>
  );
}

/** Monitor with a heartbeat trace sweeping across the screen. */
export function MachinesGlyph({ className }: GlyphProps) {
  const ref = useRef<SVGSVGElement>(null);
  const { playing, endCycle } = useRowHoverPlayback(ref);
  return (
    <svg ref={ref} {...SVG_ATTRS} className={className} data-playing={playing || undefined}>
      <rect width="20" height="14" x="2" y="3" rx="2" />
      <line x1="8" x2="16" y1="21" y2="21" />
      <line x1="12" x2="12" y1="17" y2="21" />
      <path
        className="nav-glyph-heartbeat"
        pathLength={100}
        d="M5 10h3.5l1.5-2.5 2 5.5 1.5-5 1 2h4.5"
        onAnimationIteration={endCycle}
      />
    </svg>
  );
}

/* The top-right tile of the grid morphs square -> circle -> triangle ->
   square, spinning through each morph. Every shape is sampled as the same
   24 rays from its centre (15deg apart, so square and triangle corners land
   on a ray), which makes a morph a per-point lerp. Each spin ends on one of
   the arriving shape's own symmetries, so restarting the next morph at 0deg
   is not a visible jump. Driven from JS because Safari cannot animate `d`. */

const TILE_CENTER = { x: 17.5, y: 6.5 };
const RAYS = 24;
const MORPH_MS = 650;
const HOLD_MS = 350;

type Shape = readonly { readonly x: number; readonly y: number }[];

function sampleShape(radius: (theta: number) => number, dy = 0): Shape {
  return Array.from({ length: RAYS }, (_, i) => {
    const theta = (i * 2 * Math.PI) / RAYS;
    const r = radius(theta);
    return { x: r * Math.cos(theta), y: dy + r * Math.sin(theta) };
  });
}

const SQUARE = sampleShape(
  (t) => 3.5 / Math.max(Math.abs(Math.cos(t)), Math.abs(Math.sin(t))),
);
const CIRCLE = sampleShape(() => 3.9);
// Point-up equilateral triangle, circumradius 4.4; edge normals sit opposite
// the vertices (-90, 30, 150deg). Nudged down so it sits optically centred.
const TRIANGLE_NORMALS = [90, 210, 330].map((deg) => (deg * Math.PI) / 180);
const TRIANGLE = sampleShape(
  (t) =>
    Math.min(
      ...TRIANGLE_NORMALS.map((n) => Math.cos(t - n))
        .filter((c) => c > 1e-6)
        .map((c) => 2.2 / c),
    ),
  0.6,
);

const MORPHS: readonly { from: Shape; to: Shape; spinDeg: number }[] = [
  { from: SQUARE, to: CIRCLE, spinDeg: 180 },
  { from: CIRCLE, to: TRIANGLE, spinDeg: 240 },
  { from: TRIANGLE, to: SQUARE, spinDeg: 180 },
];
const STEP_MS = MORPH_MS + HOLD_MS;
const CYCLE_MS = STEP_MS * MORPHS.length;

function tilePath(elapsedInCycle: number): string {
  const step = Math.min(Math.floor(elapsedInCycle / STEP_MS), MORPHS.length - 1);
  const morph = MORPHS[step];
  if (!morph) return "";
  const { from, to, spinDeg } = morph;
  const t = easeInOutCubic(Math.min((elapsedInCycle - step * STEP_MS) / MORPH_MS, 1));
  const angle = (spinDeg * t * Math.PI) / 180;
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  return (
    from
      .map((a, i) => {
        const b = to[i] ?? a;
        const x = a.x + (b.x - a.x) * t;
        const y = a.y + (b.y - a.y) * t;
        const px = TILE_CENTER.x + x * cos - y * sin;
        const py = TILE_CENTER.y + x * sin + y * cos;
        return `${i === 0 ? "M" : "L"}${px.toFixed(2)} ${py.toFixed(2)}`;
      })
      .join("") + "Z"
  );
}

/** Four-tile grid whose top-right tile morphs and spins. */
export function PluginsGlyph({ className }: GlyphProps) {
  const ref = useRef<SVGSVGElement>(null);
  const tileRef = useRef<SVGPathElement>(null);
  const playing = useHoverFrameLoop(ref, CYCLE_MS, (ms) => {
    tileRef.current?.setAttribute("d", tilePath(ms));
  });

  return (
    <svg ref={ref} {...SVG_ATTRS} className={className}>
      <rect width="7" height="7" x="3" y="3" rx="1" />
      {playing ? (
        <path ref={tileRef} d={tilePath(0)} />
      ) : (
        <rect width="7" height="7" x="14" y="3" rx="1" />
      )}
      <rect width="7" height="7" x="14" y="14" rx="1" />
      <rect width="7" height="7" x="3" y="14" rx="1" />
    </svg>
  );
}


/* The shield's tick folds into a single diagonal stroke, the stroke splits
   into a cross as one arm swings a quarter turn, the cross spins once, and
   the whole thing reverses back to the tick. The tick is drawn as its two
   strokes meeting at the elbow (round caps make that indistinguishable from
   lucide's single polyline). The cross has quarter-turn symmetry, so the
   full spin ends exactly where it started. */

type Segment = readonly [number, number, number, number];

const MARK_CENTER = 12;
const TICK: readonly [Segment, Segment] = [
  [9, 12, 11, 14],
  [11, 14, 15, 10],
];
// The single stroke, which is also the cross's fixed arm.
const STROKE: Segment = [9.5, 14.5, 14.5, 9.5];

// Phase durations in order.
const MARK_PHASES_MS = {
  toStroke: 500,
  holdStroke: 250,
  toCross: 500,
  holdCross: 350,
  spin: 900,
  toStrokeAgain: 500,
  holdStrokeAgain: 250,
  toTick: 500,
  holdTick: 450,
} as const;
const MARK_CYCLE_MS = Object.values(MARK_PHASES_MS).reduce((sum, ms) => sum + ms, 0);

interface MarkState {
  /** 0 = tick, 1 = single stroke. */
  readonly fold: number;
  /** Quarter turns of the swinging arm: 0 = on the stroke, 1 = crossed. */
  readonly split: number;
  readonly spinDeg: number;
}

const TICK_STATE: MarkState = { fold: 0, split: 0, spinDeg: 0 };

// Each phase maps its own eased progress (0..1) to a state.
const MARK_STEPS: readonly (readonly [number, (t: number) => MarkState])[] = [
  [MARK_PHASES_MS.toStroke, (t) => ({ fold: t, split: 0, spinDeg: 0 })],
  [MARK_PHASES_MS.holdStroke, () => ({ fold: 1, split: 0, spinDeg: 0 })],
  [MARK_PHASES_MS.toCross, (t) => ({ fold: 1, split: t, spinDeg: 0 })],
  [MARK_PHASES_MS.holdCross, () => ({ fold: 1, split: 1, spinDeg: 0 })],
  [MARK_PHASES_MS.spin, (t) => ({ fold: 1, split: 1, spinDeg: 360 * t })],
  [MARK_PHASES_MS.toStrokeAgain, (t) => ({ fold: 1, split: 1 - t, spinDeg: 0 })],
  [MARK_PHASES_MS.holdStrokeAgain, () => ({ fold: 1, split: 0, spinDeg: 0 })],
  [MARK_PHASES_MS.toTick, (t) => ({ fold: 1 - t, split: 0, spinDeg: 0 })],
];

function markState(ms: number): MarkState {
  let rest = ms;
  for (const [duration, state] of MARK_STEPS) {
    if (rest < duration) return state(easeInOutCubic(rest / duration));
    rest -= duration;
  }
  return TICK_STATE;
}

function rotate(x: number, y: number, deg: number): [number, number] {
  const angle = (deg * Math.PI) / 180;
  const dx = x - MARK_CENTER;
  const dy = y - MARK_CENTER;
  return [
    MARK_CENTER + dx * Math.cos(angle) - dy * Math.sin(angle),
    MARK_CENTER + dx * Math.sin(angle) + dy * Math.cos(angle),
  ];
}

function markPath({ fold, split, spinDeg }: MarkState): string {
  return TICK.map(([tx1, ty1, tx2, ty2], i) => {
    const [sx1, sy1, sx2, sy2] = STROKE;
    // Fold: lerp each tick stroke onto the single stroke.
    let a: [number, number] = [tx1 + (sx1 - tx1) * fold, ty1 + (sy1 - ty1) * fold];
    let b: [number, number] = [tx2 + (sx2 - tx2) * fold, ty2 + (sy2 - ty2) * fold];
    // Split: the first stroke swings a quarter turn about the centre into
    // the cross's other arm (rotating rather than lerping keeps its length).
    const deg = (i === 0 ? 90 * split : 0) + spinDeg;
    a = rotate(a[0], a[1], deg);
    b = rotate(b[0], b[1], deg);
    return `M${a[0].toFixed(2)} ${a[1].toFixed(2)}L${b[0].toFixed(2)} ${b[1].toFixed(2)}`;
  }).join("");
}

/** Shield whose tick turns into a spinning cross and back. */
export function ApprovalsGlyph({ className }: GlyphProps) {
  const ref = useRef<SVGSVGElement>(null);
  const markRef = useRef<SVGPathElement>(null);
  useHoverFrameLoop(ref, MARK_CYCLE_MS, (ms) => {
    markRef.current?.setAttribute("d", markPath(markState(ms)));
  });
  return (
    <svg ref={ref} {...SVG_ATTRS} className={className}>
      <path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z" />
      <path ref={markRef} d={markPath(TICK_STATE)} />
    </svg>
  );
}
