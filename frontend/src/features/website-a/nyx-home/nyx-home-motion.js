/* NyxID home page animations, ported from the "light night" design
   (mockups/landing-combined/nyxid-light-night.html and kit.js).

   The design's code is kept as it was, with these changes for the app:
   - GSAP, its plugins and Lenis are imported instead of read from window.
   - Everything searches this page's element, and day/night is set on it, never on <html>.
   - No HTML strings are inserted: the pile markup is rendered by React, and text that changes is built as elements.
   - startNyxHome returns a stop function that removes every listener, timer, observer, tween and pin,
     and puts the page's markup back as React rendered it, so leaving and returning starts clean. */
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';
import { SplitText } from 'gsap/SplitText';
import { ScrambleTextPlugin } from 'gsap/ScrambleTextPlugin';
import { Flip } from 'gsap/Flip';
import { Draggable } from 'gsap/Draggable';
import { InertiaPlugin } from 'gsap/InertiaPlugin';
import { DrawSVGPlugin } from 'gsap/DrawSVGPlugin';
import Lenis from 'lenis';

gsap.registerPlugin(ScrollTrigger, SplitText, ScrambleTextPlugin, Flip, Draggable, InertiaPlugin, DrawSVGPlugin);

const SVG_NS = 'http://www.w3.org/2000/svg';
function make(tag, attrs, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) el.setAttribute(k, String(v));
  el.append(...children);
  return el;
}
function svgPath(attrs) {
  const el = document.createElementNS(SVG_NS, 'path');
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  return el;
}
function checkIcon() {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS(SVG_NS, 'use');
  use.setAttribute('href', '#ck');
  svg.append(use);
  return svg;
}

/* Everything the page starts is registered here, so stop() can undo it. */
function createEnv(page) {
  const controller = new AbortController();
  const stops = [];
  let alive = true;
  return {
    signal: controller.signal,
    ctx: null,
    get alive() { return alive; },
    onStop: (fn) => stops.push(fn),
    stop() {
      if (!alive) return;
      alive = false;
      controller.abort();
      for (const fn of stops.splice(0).reverse()) {
        try { fn(); } catch { /* keep stopping the rest */ }
      }
    },
    page,
  };
}

export function startNyxHome(page) {
  const env = createEnv(page);
  /* the markup as React rendered it, restored on stop */
  const pristine = [...page.childNodes].map((n) => n.cloneNode(true));

  /* window-level hooks the design used, now tied to this page's lifetime */
  const addEventListener = (type, fn, opts) =>
    window.addEventListener(type, fn, { ...(typeof opts === 'object' ? opts : { capture: !!opts }), signal: env.signal });
  const frames = new Set();
  const requestAnimationFrame = (cb) => {
    if (!env.alive) return 0;
    const id = window.requestAnimationFrame((t) => { frames.delete(id); if (env.alive) cb(t); });
    frames.add(id);
    return id;
  };
  const timers = new Set();
  const setTimeout = (cb, ms) => {
    if (!env.alive) return 0;
    const id = window.setTimeout(() => { timers.delete(id); if (env.alive) cb(); }, ms);
    timers.add(id);
    return id;
  };
  const observers = [];
  class IntersectionObserver extends window.IntersectionObserver {
    constructor(cb, opts) { super(cb, opts); observers.push(this); }
  }
  class MutationObserver extends window.MutationObserver {
    constructor(cb) { super(cb); observers.push(this); }
  }
  env.onStop(() => {
    frames.forEach((id) => window.cancelAnimationFrame(id));
    timers.forEach((id) => window.clearTimeout(id));
    observers.forEach((o) => o.disconnect());
  });

  env.ctx = gsap.context(() => {
  /* ---------- from kit.js: the pile on the counter gets sorted ---------- */
  const initSort = (() => {
    const G = gsap;
    const has = true;
    const P = { ST: ScrollTrigger, Flip, Draggable, Inertia: InertiaPlugin, Draw: DrawSVGPlugin };
  const $ = (s, r = page) => r.querySelector(s);
  const $$ = (s, r = page) => [...r.querySelectorAll(s)];
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
  const setText = (el, t) => { if (el && el.textContent !== String(t)) el.textContent = t; };

  /* ---------- the pile gets sorted ---------- */
  function initSort(section, hooks = {}) {
    /* the pile markup is rendered by nyx-home-page.tsx */
    const pile = $('[data-pile]', section);
    const trays = $$('[data-tray]', section);
    const objs = $$('[data-obj]', section);
    const ORDER = ['slip', 'letter', 'invoice', 'receipt', 'sticky', 'call', 'cal'];
    const byId = Object.fromEntries(objs.map((o) => [o.dataset.obj, o]));
    const slot = (id) => $(`[data-slot="${id}"]`, section);
    const hole = (id) => $('.slot-hole', slot(id));
    const CHECK = 'M12 20.5l5.5 5.5L28.5 14', WAIT = 'M20 11v11M20 28v.5';
    const S = { auto: 0, manual: new Set(), decision: null, motion: false };
    const isSorted = (o) => o.classList.contains('is-sorted');

    function stamp(o) {
      o.classList.add('is-stamped');
      if (!S.motion || !has) return;
      const st = $('.stamp', o);
      G.fromTo(st, { scale: 1.7, rotate: -30 }, { scale: 1, rotate: -12, duration: 0.45, ease: 'back.out(2.2)', overwrite: true });
      if (P.Draw) G.fromTo($$('circle, path', st), { drawSVG: '0%' }, { drawSVG: '100%', duration: 0.45, stagger: 0.14, ease: 'power2.out', overwrite: true });
      const tray = o.closest('[data-tray]');
      if (tray) { tray.classList.add('is-thunk'); setTimeout(() => tray.classList.remove('is-thunk'), 160); }
    }
    function fly(o, parent, sorted, done) {
      const flip = S.motion && has && P.Flip;
      const state = flip ? P.Flip.getState(o) : null;
      if (has) G.killTweensOf(o);
      parent.appendChild(o);
      o.classList.toggle('is-sorted', sorted);
      if (has) G.set(o, { clearProps: 'transform' });
      if (flip) P.Flip.from(state, { duration: 0.72, ease: 'power3.inOut', scale: true, absolute: true, zIndex: 40, onComplete: done });
      else if (done) done();
    }
    function drag(o, on) {
      const d = has && P.Draggable && P.Draggable.get(o);
      if (d) on ? d.enable() : d.disable();
    }
    function sortObj(id) {
      const o = byId[id];
      drag(o, false);
      o.tabIndex = -1;
      fly(o, hole(id), true, () => { slot(id).classList.add('is-full'); stamp(o); });
      if (o.dataset.dest === 'wait') showWait();
      if (hooks.onSort) hooks.onSort(id, o);
    }
    function unsortObj(id) {
      const o = byId[id];
      if (id === 'invoice') resetDecision();
      slot(id).classList.remove('is-full');
      o.classList.remove('is-stamped', 'is-approved');
      o.tabIndex = 0;
      fly(o, pile, false, () => drag(o, true));
    }
    function apply() {
      ORDER.forEach((id, i) => {
        const want = i < S.auto || S.manual.has(id);
        const o = byId[id];
        if (want && !isSorted(o)) sortObj(id);
        else if (!want && isSorted(o)) unsortObj(id);
      });
      counts();
    }
    function counts() {
      const sorted = objs.filter(isSorted);
      const inv = isSorted(byId.invoice);
      const done = sorted.filter((o) => o.dataset.dest === 'done').length + (inv && S.decision === 'approved' ? 1 : 0);
      const wait = inv && !S.decision ? 1 : 0;
      setText($('[data-n-done]', section), done);
      setText($('[data-n-wait]', section), wait);
      setText($('[data-h-done]', section), done ? `${done} of 7` : 'nothing yet');
      setText($('[data-h-wait]', section), wait ? '1 thing' : inv ? 'nothing now' : 'nothing yet');
      pile.classList.toggle('is-empty', sorted.length === objs.length);
      if (hooks.onChange) hooks.onChange({ done, wait, sorted: sorted.length, total: objs.length });
    }
    const hand = (id) => { S.manual.add(id); apply(); };

    /* the one thing that waits: approve, deny, undo */
    const waitSlot = slot('invoice'), doneSlot = slot('invoice-done');
    const waitText = $('[data-wait-text]', section), waitNone = $('[data-wait-none]', section);
    const bOk = $('[data-approve]', section), bNo = $('[data-deny]', section), bUndo = $('[data-undo]', section);
    const WAIT_NODES = [...waitText.childNodes].map((n) => n.cloneNode(true));
    function showWait() { waitSlot.hidden = false; waitNone.hidden = true; }
    function resetDecision() {
      S.decision = null;
      waitText.replaceChildren(...WAIT_NODES.map((n) => n.cloneNode(true)));
      bOk.hidden = bNo.hidden = false; bUndo.hidden = true;
      doneSlot.hidden = true; doneSlot.classList.remove('is-full');
      waitSlot.hidden = false; waitNone.hidden = true;
      $('.stamp path', byId.invoice).setAttribute('d', WAIT);
      byId.invoice.classList.remove('is-approved');
    }
    bOk.addEventListener('click', () => {
      if (!isSorted(byId.invoice)) return;
      S.decision = 'approved';
      const o = byId.invoice;
      doneSlot.hidden = false;
      o.classList.remove('is-stamped');
      $('.stamp path', o).setAttribute('d', CHECK);
      o.classList.add('is-approved');
      fly(o, $('.slot-hole', doneSlot), true, () => { doneSlot.classList.add('is-full'); stamp(o); });
      waitSlot.hidden = true; waitNone.hidden = false;
      counts();
      if (hooks.onApprove) hooks.onApprove(o);
    });
    bNo.addEventListener('click', () => {
      S.decision = 'denied';
      waitText.replaceChildren(make('b', null, 'Denied.'), ' NyxBot sent nothing to Harbour Fish Co. The draft is still there if you change your mind.');
      bOk.hidden = bNo.hidden = true; bUndo.hidden = false;
      counts();
    });
    bUndo.addEventListener('click', () => {
      waitText.replaceChildren(...WAIT_NODES.map((n) => n.cloneNode(true)));
      S.decision = null;
      bOk.hidden = bNo.hidden = false; bUndo.hidden = true;
      counts();
    });

    /* keyboard path: Enter or Space on a slip hands it to NyxBot */
    objs.forEach((o) => o.addEventListener('click', (e) => { if (e.detail === 0 && !isSorted(o)) hand(o.dataset.obj); }));

    function staticAll() { S.motion = false; S.auto = ORDER.length; apply(); }
    function makeDraggable() {
      if (!P.Draggable) return;
      const over = (d) => trays.some((t) => d.hitTest(t, '25%'));
      const light = (on) => trays.forEach((t) => t.classList.toggle('is-over', on));
      objs.forEach((o) => {
        if (P.Draggable.get(o)) return;
        const id = o.dataset.obj;
        const back = () => G.to(o, { x: 0, y: 0, duration: 0.7, ease: 'elastic.out(1,0.75)' });
        P.Draggable.create(o, {
          type: 'x,y', bounds: section, inertia: !!P.Inertia, zIndexBoost: true, minimumMovement: 4,
          onDrag() { light(over(this)); },
          onThrowUpdate() { light(over(this)); },
          onDragEnd() {
            if (over(this)) { if (this.tween) this.tween.kill(); light(false); hand(id); }
            else if (!this.tween) { light(false); back(); }
          },
          onThrowComplete() { light(false); if (over(this)) hand(id); else back(); },
          onClick() { hand(id); },
        });
      });
    }

    const api = { section, objs, byId, ORDER, isSorted, prepare: null };
    if (has && P.ST) {
      const mm = G.matchMedia();
      mm.add('(min-width: 960px) and (prefers-reduced-motion: no-preference)', () => {
        S.motion = false; S.auto = 0; S.manual.clear(); apply();
        S.motion = true;
        makeDraggable();
        objs.forEach((o) => drag(o, !isSorted(o)));
        const p = { v: 0 };
        /* optional per page: hooks.pace(progress) -> how many slips are sorted, hooks.pinEnd -> pin length.
           Pages that pass neither keep the original pacing. */
        const pace = hooks.pace || ((v) => Math.floor(v * 8.7 - 0.75));
        G.to(p, {
          v: 1, ease: 'none',
          onUpdate() {
            const n = clamp(pace(p.v), 0, ORDER.length);
            if (n !== S.auto) { S.auto = n; apply(); }
          },
          scrollTrigger: { trigger: section, start: 'top top', end: hooks.pinEnd || '+=240%', pin: true, scrub: 0.3,
            onUpdate: (st) => hooks.onProgress && hooks.onProgress(st.progress) },
        });
        if (hooks.onMotion) hooks.onMotion(api);
        return () => { objs.forEach((o) => drag(o, false)); staticAll(); };
      });
      mm.add('(max-width: 959px), (prefers-reduced-motion: reduce)', () => staticAll());
    } else {
      staticAll();
    }
    return api;
  }

    return initSort;
  })();

  const $ = (s, r = page) => r.querySelector(s);
  const $$ = (s, r = page) => [...r.querySelectorAll(s)];
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const setText = (el, t) => { if (el && el.textContent !== String(t)) el.textContent = t; };
  const hasGsap = true;
  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const Split = SplitText, Scramble = ScrambleTextPlugin;
  let lenis = null;

  /* ---------- smooth scroll ---------- */
  if (hasGsap) {
    gsap.registerPlugin(...[ScrollTrigger, Split, Scramble].filter(Boolean));
    if (!reduce) {
      lenis = new Lenis({ lerp: 0.1, smoothWheel: true, wheelMultiplier: 1 });
      lenis.on('scroll', ScrollTrigger.update);
      const lenisRaf = (t) => lenis.raf(t * 1000);
      gsap.ticker.add(lenisRaf);
      gsap.ticker.lagSmoothing(0);
      const smooth = lenis;
      env.onStop(() => { gsap.ticker.remove(lenisRaf); gsap.ticker.lagSmoothing(500, 33); smooth.destroy(); });
    }
  }
  const scrollToY = (y) => {
    if (lenis) lenis.scrollTo(y, { duration: 0.9 });
    else window.scrollTo({ top: y, behavior: reduce ? 'auto' : 'smooth' });
  };
  $$('a[href^="#"]').forEach((a) => a.addEventListener('click', (e) => {
    const target = $(a.getAttribute('href'));
    if (!target) return;
    e.preventDefault();
    const st = hasGsap && ScrollTrigger.getAll().find((s) => s.trigger === target && s.pin);
    scrollToY(st ? st.start : target.getBoundingClientRect().top + scrollY);
  }));

  /* ---------- 1. hero: the chrome N, the falling requests, the done ---------- */
  const hero = $('[data-hero]');
  requestAnimationFrame(() => requestAnimationFrame(() => hero.classList.add('is-in')));
  const nstage = $('[data-nstage]'), mark = $('[data-mark]'), canvas = $('canvas', mark);
  const heroCopy = $('.hero-copy'), ticks = $('[data-ticks]'), countEl = $('[data-count]');
  /* ap: how far the requests have gone into the N, 0 -> 1. The pinned scroll drives it on a desktop. */
  let ap = 0;

  /* the chrome NyxID mark: the logo extruded and raymarched, two liquid drops, a light studio */
  const FRAG = `
precision highp float;
uniform vec2 uRes; uniform float uTime; uniform vec2 uMouse; uniform float uSpin;
mat3 R;
float smin(float a,float b,float k){float h=clamp(.5+.5*(b-a)/k,0.,1.);return mix(b,a,h)-k*h*(1.-h);}
float sdTri(vec2 p,vec2 p0,vec2 p1,vec2 p2){
  vec2 e0=p1-p0,e1=p2-p1,e2=p0-p2,v0=p-p0,v1=p-p1,v2=p-p2;
  vec2 q0=v0-e0*clamp(dot(v0,e0)/dot(e0,e0),0.,1.);
  vec2 q1=v1-e1*clamp(dot(v1,e1)/dot(e1,e1),0.,1.);
  vec2 q2=v2-e2*clamp(dot(v2,e2)/dot(e2,e2),0.,1.);
  float s=sign(e0.x*e2.y-e0.y*e2.x);
  vec2 d=min(min(vec2(dot(q0,q0),s*(v0.x*e0.y-v0.y*e0.x)),vec2(dot(q1,q1),s*(v1.x*e1.y-v1.y*e1.x))),vec2(dot(q2,q2),s*(v2.x*e2.y-v2.y*e2.x)));
  return -sqrt(d.x)*sign(d.y);
}
float mark(vec2 p){
  float rc=p.x>0.?.41:.02;
  vec2 q=abs(p)-vec2(1.-rc);
  float d=min(max(q.x,q.y),0.)+length(max(q,0.))-rc;
  d=max(d,-sdTri(p,vec2(-.332,1.3),vec2(-.332,-.14),vec2(.509,1.3)));
  d=max(d,-sdTri(p,vec2(0.,.136),vec2(0.,-1.3),vec2(-.843,-1.3)));
  return d;
}
float logo(vec3 p){
  const float S=.66,H=.22,E=.09;
  vec3 q=(R*p)/S;
  vec2 w=vec2(mark(q.xy)+E,abs(q.z)-H+E);
  return (min(max(w.x,w.y),0.)+length(max(w,0.))-E)*S;
}
float map(vec3 p){
  vec3 c3=vec3(uMouse*.75,.36);
  float a=uTime*.55;
  vec3 c4=vec3(cos(a)*1.0,sin(a*1.7)*.25,sin(a)*.45);
  float d=logo(p);
  d=smin(d,length(p-c3)-.17,.25);
  d=smin(d,length(p-c4)-.12,.25);
  return d;
}
vec3 nrm(vec3 p){vec2 e=vec2(.0015,0.);return normalize(vec3(map(p+e.xyy)-map(p-e.xyy),map(p+e.yxy)-map(p-e.yxy),map(p+e.yyx)-map(p-e.yyx)));}
vec3 env(vec3 r){
  float y=r.y;
  vec3 sky=mix(vec3(.82,.83,.87),vec3(1.),smoothstep(0.,.55,y));
  vec3 gnd=mix(vec3(.12,.12,.14),vec3(.62,.63,.67),smoothstep(-.05,-.75,y));
  vec3 col=y>0.?sky:gnd;
  col=mix(col,vec3(.05,.05,.07),exp(-abs(y)*18.)*.92);
  float a=atan(r.x,r.z);
  col*=1.-.55*smoothstep(.22,0.,abs(a-1.9))*smoothstep(-.1,.5,y);
  col+=vec3(.9)*smoothstep(.04,0.,abs(y-.32))*smoothstep(.9,.2,abs(a))*.35;
  col+=vec3(1.)*smoothstep(.07,0.,abs(a-.85))*smoothstep(.05,.45,y)*.9;
  col+=vec3(1.)*smoothstep(.05,0.,abs(a+.4))*smoothstep(.2,.7,y)*.5;
  col+=vec3(.49,.36,.88)*smoothstep(.16,0.,abs(a+1.7))*smoothstep(-.3,.35,y)*1.1;
  col+=vec3(.45,.72,1.)*smoothstep(.12,0.,abs(a-2.5))*.35;
  return col;
}
void main(){
  vec2 uv=(gl_FragCoord.xy-.5*uRes)/uRes.y;
  float ry=sin(uTime*.35)*.5+uMouse.x*.45+uSpin, rx=sin(uTime*.27+1.)*.18-uMouse.y*.3;
  float cy=cos(ry),sy=sin(ry),cx=cos(rx),sx=sin(rx);
  R=mat3(cy,0.,sy,0.,1.,0.,-sy,0.,cy)*mat3(1.,0.,0.,0.,cx,-sx,0.,sx,cx);
  vec3 ro=vec3(0.,0.,3.6),rd=normalize(vec3(uv,-1.45));
  /* edge smoothing: remember how close each ray came to the surface, measured in pixels at that depth;
     a ray that just misses still draws the surface, partly transparent, so outlines are smooth instead of stepped */
  float pf=1./(uRes.y*1.45);
  float t=0.,d=1.,bestR=1e9,bestT=0.;
  for(int i=0;i<120;i++){d=map(ro+rd*t);float rr=d/(t*pf+1e-4);if(rr<bestR){bestR=rr;bestT=t;}if(d<.0008||t>8.)break;t+=d;}
  float alpha=1.;
  if(d>.0008){if(bestR>1.2){gl_FragColor=vec4(0.);return;}t=bestT;alpha=smoothstep(1.2,0.,bestR);}
  vec3 p=ro+rd*t,n=nrm(p),r=reflect(rd,n);
  vec3 col=env(r);
  float fr=pow(1.-max(dot(n,-rd),0.),3.);
  /* a little more light where the surface turns away from the viewer, so the rims are not black outlines */
  col=mix(col*.94,vec3(1.),fr*.34);
  gl_FragColor=vec4(col,alpha);
}`;
  const VERT = 'attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}';
  function startGL() {
    const gl = canvas.getContext('webgl', { alpha: true, premultipliedAlpha: false, antialias: false });
    if (!gl) return false;
    const sh = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null; };
    const vs = sh(gl.VERTEX_SHADER, VERT), fs = sh(gl.FRAGMENT_SHADER, FRAG);
    if (!vs || !fs) return false;
    const prog = gl.createProgram(); gl.attachShader(prog, vs); gl.attachShader(prog, fs); gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) return false;
    gl.useProgram(prog);
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, 'p'); gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    const uRes = gl.getUniformLocation(prog, 'uRes'), uTime = gl.getUniformLocation(prog, 'uTime'), uMouse = gl.getUniformLocation(prog, 'uMouse'), uSpin = gl.getUniformLocation(prog, 'uSpin');
    const target = { x: 0, y: 0 }, m = { x: 0, y: 0 };
    let lastMove = -1e9, visible = true, lost = false, t = 6, last = performance.now(), raf = 0, spin = 0;
    function size() {
      /* render at 1.6x the screen pixels (at most 2.4), then let the browser scale it down: smoother reflections */
      const dpr = Math.min((devicePixelRatio || 1) * 1.6, 2.4), r = canvas.getBoundingClientRect();
      canvas.width = Math.max(1, Math.round(r.width * dpr)); canvas.height = Math.max(1, Math.round(r.height * dpr));
      gl.viewport(0, 0, canvas.width, canvas.height);
    }
    function frame(now) {
      raf = 0;
      if (lost) return;
      const dt = Math.min(0.05, (now - last) / 1000); last = now;
      if (!reduce) t += dt;
      if (now - lastMove > 2500) { target.x = Math.sin(t * 0.6) * 0.45; target.y = Math.cos(t * 0.45) * 0.35; }
      m.x += (target.x - m.x) * 0.06; m.y += (target.y - m.y) * 0.06;
      /* every request that goes in turns the N a little further */
      spin += (ap * 6.2832 - spin) * 0.12;
      gl.uniform2f(uRes, canvas.width, canvas.height); gl.uniform1f(uTime, t); gl.uniform2f(uMouse, m.x, m.y); gl.uniform1f(uSpin, spin);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      if (visible && !reduce) raf = requestAnimationFrame(frame);
    }
    const kick = () => { if (!raf && !lost) { last = performance.now(); raf = requestAnimationFrame(frame); } };
    hero.addEventListener('pointermove', (e) => {
      const r = hero.getBoundingClientRect();
      target.x = clamp(((e.clientX - r.left) / r.width) * 2 - 1, -1, 1);
      target.y = clamp(-(((e.clientY - r.top) / r.height) * 2 - 1), -1, 1);
      lastMove = performance.now();
      if (reduce) kick();
    }, { passive: true });
    /* a lost GPU context hands the stage back to the CSS chrome mark */
    canvas.addEventListener('webglcontextlost', (e) => { e.preventDefault(); lost = true; mark.classList.remove('gl-on'); });
    new IntersectionObserver(([en]) => { visible = en.isIntersecting; if (visible) kick(); }).observe(hero);
    addEventListener('resize', () => { size(); kick(); });
    size(); kick();
    mark.classList.add('gl-on');
    return true;
  }
  try { startGL(); } catch (e) { mark.classList.remove('gl-on'); }

  /* the falling requests: the jobs NyxBot and your agents send through NyxID. They drift down the right side
     and shy away from the pointer; ap pulls them into the N one by one. */
  const box = $('[data-rain]'), kinds = $('#kinds');
  const KINDS = ['call', 'cal', 'letter', 'sticky', 'invoice', 'slip', 'receipt'];
  const rand = (a, b) => a + Math.random() * (b - a);
  /* N: how many requests fall (10, was 12). LANES: the columns they are dealt into across the right side, so they
     spread out and do not bunch into one vertical line. SLIP_SCALE: the slips are 10% smaller than they were. */
  const N = 10, LANES = 5, SLIP_SCALE = 0.9, slips = [];
  setText($('[data-total]'), N);
  /* slip k starts at ap*1.05 = k and is fully inside 0.3 later; the last one lands at ap = 0.93 */
  const absorbed = (k) => clamp((ap * 1.05 - k) / 0.3, 0, 1);
  let W = 0, H = 0, tx = 0, ty = 0, rainOn = false, rLast = 0, mx = -9999, my = -9999, lastY = scrollY;
  /* wallX: where the headline column ends (plus a 48px gap), in the slip layer's own coordinates. On wide screens
     slips never start left of it, are eased back if the pointer pushes them past it, and fade as they near it. */
  let wallX = 0;
  const measure = () => {
    W = box.clientWidth; H = box.clientHeight;
    const bx = box.getBoundingClientRect();
    let right = 0;
    for (const el of heroCopy.children) right = Math.max(right, el.getBoundingClientRect().right);
    wallX = right ? right - bx.left + 48 : 0.44 * W;
  };
  const aim = () => { const bx = box.getBoundingClientRect(), r = mark.getBoundingClientRect(); tx = r.left - bx.left + r.width / 2; ty = r.top - bx.top + r.height / 2; };
  const wide = () => innerWidth >= 960;
  /* x is a slip's left edge. The slips share the stretch from the headline's wall to the right side: where the side
     navigator shows (>=1180px) a slip's right edge stays 110px clear of the screen edge, just short of the navigator's
     label. That stretch is cut into LANES columns; a slip sits in its lane with a little random drift, so two slips
     in different lanes can never line up one above the other. */
  const laneX = (lane, w = 0) => {
    const lo = wide() ? wallX : 0.02 * W;
    const hi = Math.max(lo, (wide() ? (innerWidth >= 1180 ? W - 110 : W * 0.99) : 0.98 * W) - w);
    return lo + (hi - lo) * clamp((lane + rand(-0.28, 0.28)) / (LANES - 1), 0, 1);
  };
  /* a slip that has fallen off the bottom comes back in the lane with the most empty room at the top */
  const freeLane = (self) => {
    let best = 0, bestGap = -1;
    for (let l = 0; l < LANES; l++) {
      let gap = 1e6;
      for (const s of slips) if (s !== self && s.lane === l) gap = Math.min(gap, Math.abs(s.y + 180));
      gap += rand(0, 60); /* near-equal lanes take turns */
      if (gap > bestGap) { bestGap = gap; best = l; }
    }
    return best;
  };
  /* 1 well right of the wall, easing to 0 at the wall itself */
  const wallFade = (x) => { if (!wide()) return 1; const t = clamp((x - wallX) / 70, 0, 1); return t * t * (3 - 2 * t); };
  measure();
  for (let i = 0; i < N; i++) {
    const el = document.createElement('div');
    el.className = 'rain-slip';
    const z = 0.5 + ((i * 7) % N) / N * 0.5;
    el.style.fontSize = `${((7 + z * 8) * SLIP_SCALE).toFixed(1)}px`;
    el.style.zIndex = Math.round(z * 10);
    if (z < 0.66) el.style.filter = 'blur(1.2px)';
    el.appendChild(kinds.querySelector(`[data-k="${KINDS[i % KINDS.length]}"]`).cloneNode(true));
    box.appendChild(el);
    /* dealt out evenly: each slip takes the lane two along from the last one (0 2 4 1 3 ...), and they start evenly
       spaced from above the top to the bottom, so neighbours in height are far apart in width */
    const lane = (i * 2) % LANES;
    slips.push({ el, z, lane, op: 0.4 + z * 0.55, k: (i / N) * 0.75, w: el.offsetWidth, h: el.offsetHeight, x: laneX(lane, el.offsetWidth), y: (-0.12 + ((i + rand(-0.25, 0.25)) / N) * 1.14) * H, vy: 18 + z * 22, vx: 0, ph: rand(0, 6.28), sw: rand(0.35, 0.8) });
  }
  const place = (s) => {
    const a = absorbed(s.k), e = a * a * (3 - 2 * a);
    const x = s.x + (tx - s.w / 2 - s.x) * e, y = s.y + (ty - s.h / 2 - s.y) * e;
    /* on the way in a slip shrinks, turns and fades, as if the N swallows it */
    s.el.style.transform = `translate3d(${x.toFixed(1)}px,${y.toFixed(1)}px,0) scale(${(1 - 0.9 * e).toFixed(3)}) rotate(${(e * 140 * (s.ph > 3.14 ? 1 : -1)).toFixed(1)}deg)`;
    s.el.style.opacity = (s.op * (1 - e * e) * wallFade(x)).toFixed(3);
  };
  const placeAll = () => slips.forEach(place);
  aim();
  setTimeout(() => { measure(); aim(); placeAll(); }, 1400); /* fonts have loaded by now, so the wall is measured again */
  placeAll();
  addEventListener('resize', () => { measure(); aim(); placeAll(); });
  if (!reduce) {
    const rainFrame = (now) => {
      if (!rainOn) return;
      const dt = Math.min(0.05, (now - rLast) / 1000 || 0.016); rLast = now;
      const boost = 1 + Math.min(4, Math.abs(scrollY - lastY) * 0.12); lastY = scrollY;
      for (const s of slips) {
        const cx = s.x + 70 * s.z, cy = s.y + 45 * s.z, dx = cx - mx, dy = cy - my, d = Math.hypot(dx, dy);
        if (d < 190) { const f = (190 - d) / 190; s.vx += (dx / (d || 1)) * f * 520 * dt; }
        /* past the wall, a gentle spring eases the slip back out, so it drifts rather than bounces */
        if (wide() && s.x < wallX + 20) s.vx += (wallX + 20 - s.x) * 6 * dt;
        /* the same on the right where the side navigator shows: a swaying slip is eased back before it reaches the navigator's label */
        if (innerWidth >= 1180 && s.x + s.w > W - 110) s.vx -= (s.x + s.w - (W - 110)) * 6 * dt;
        s.vx *= 0.94;
        s.y += s.vy * boost * dt;
        s.x += (Math.sin(now * 0.001 * s.sw + s.ph) * 16 * s.z + s.vx) * dt;
        if (s.y > H + 40) { s.y = -180; s.lane = freeLane(s); s.x = laneX(s.lane, s.w); s.vx = 0; }
        place(s);
      }
      requestAnimationFrame(rainFrame);
    };
    hero.addEventListener('pointermove', (e) => { const r = box.getBoundingClientRect(); mx = e.clientX - r.left; my = e.clientY - r.top; });
    hero.addEventListener('pointerleave', () => { mx = my = -9999; });
    new IntersectionObserver(([en]) => { const was = rainOn; rainOn = en.isIntersecting; if (rainOn && !was) { rLast = performance.now(); requestAnimationFrame(rainFrame); } }).observe(hero);
  }

  /* each request that lands makes the N glow once and fills the arc; after the last one NyxBot says "Done!" */
  let inCount = -1, sayT = 0;
  /* heroPinned: true only while the desktop pin drives the hero. The headline's small upward drift belongs to that
     pinned scroll; on a phone it would push the headline up under the nav. */
  let heroPinned = false;
  const doneChat = $('[data-done-chat]');
  function applyHero() {
    nstage.style.setProperty('--p', ap.toFixed(3));
    nstage.style.setProperty('--fill', (slips.reduce((t, s) => t + absorbed(s.k), 0) / N).toFixed(3));
    heroCopy.style.transform = heroPinned ? `translateY(${(-24 * ap).toFixed(1)}px)` : '';
    ticks.style.transform = `rotate(${(ap * 120).toFixed(1)}deg)`;
    const n = slips.filter((s) => absorbed(s.k) >= 1).length;
    if (n !== inCount) {
      if (n > inCount && inCount >= 0) { mark.classList.remove('is-gulp'); void mark.offsetWidth; mark.classList.add('is-gulp'); }
      inCount = n;
      setText(countEl, n);
      const done = n === N;
      nstage.classList.toggle('is-done', done);
      hero.classList.toggle('is-done', done);
      clearTimeout(sayT);
      if (!done) doneChat.classList.remove('is-said');
      else if (reduce) doneChat.classList.add('is-said');
      else sayT = setTimeout(() => doneChat.classList.add('is-said'), 260);
    }
    if (!rainOn) placeAll();
  }
  applyHero();
  /* scroll sets apT; ap follows it a little each frame, so the pull eases in and out instead of stepping with every wheel tick */
  let apT = 0;
  const follow = () => {
    const d = apT - ap;
    if (d === 0) return;
    ap = Math.abs(d) < 0.0005 ? apT : ap + d * 0.18;
    applyHero();
  };
  gsap.ticker.add(follow);
  env.onStop(() => gsap.ticker.remove(follow));


  /* pinned on desktop, finished state everywhere else.
     The hero is pinned for 165% of a screen: the requests go into the N over the first 80%,
     "Done!" holds for the next 85% so it can be read, then the run scene slides up over it like the other sections. */
  const HERO_PIN = 1.65, ABSORB_END = 0.8 / HERO_PIN;
  function staticMode() {
    if (reduce || !hasGsap) { ap = apT = 1; applyHero(); return () => {}; }
    /* no pin: the requests fall into the N once, on their own, when it comes into view */
    ap = apT = 0; applyHero();
    let tw = null;
    const io = new IntersectionObserver(([en]) => {
      if (!en.isIntersecting) return;
      io.disconnect();
      const o = { p: 0 };
      tw = gsap.to(o, { p: 1, duration: 3.6, delay: 0.6, ease: 'power1.inOut', onUpdate: () => { apT = o.p; } });
    }, { threshold: 0.5 });
    io.observe(nstage);
    return () => { io.disconnect(); if (tw) tw.kill(); };
  }

  if (hasGsap) {
    const mm = gsap.matchMedia();
    mm.add('(min-width: 960px) and (prefers-reduced-motion: no-preference)', () => {
      heroPinned = true;
      ap = apT = 0; applyHero();
      ScrollTrigger.create({ trigger: $('#top'), start: 'top top', end: `+=${HERO_PIN * 100}%`, pin: true, anticipatePin: 1,
        onUpdate: (st) => { apT = clamp(st.progress / ABSORB_END, 0, 1); }, onRefresh: () => { aim(); placeAll(); } });
      return () => { heroPinned = false; applyHero(); };
    });
    /* how it works: the pile (kit.js) pins after the hero and before Solutions.
       Sorting starts the moment the pin does, all seven are in by 72%, and the finished counter holds for the rest. */
    const pileHooks = {
      pinEnd: '+=260%',
      pace: (v) => Math.ceil((v - 0.02) / 0.7 * 7),
      onChange: ({ done, wait }) => {
        const msg = $('#pile .pile-empty');
        if (msg) msg.replaceChildren(make('i', null, checkIcon()), make('span', null, `Morning. ${done} ${done === 1 ? 'thing is' : 'things are'} done`, ...(wait ? [', and ', make('em', null, '1 is waiting for your yes'), '.'] : ['.'])));
      },
    };
    {
      initSort($('#pile'), pileHooks);
      /* the slips land on the counter as the section arrives, so the pile is already alive before the pin */
      if (matchMedia('(min-width: 960px) and (prefers-reduced-motion: no-preference)').matches) {
        /* animate the inner layer: kit.js stops tweens on the slip itself when it flies, which would freeze a half-faded slip */
        gsap.from($$('#pile .obj-in'), { yPercent: -70, opacity: 0, duration: 0.75, ease: 'back.out(1.6)', stagger: 0.07, clearProps: 'transform,opacity',
          scrollTrigger: { trigger: $('#pile'), start: 'top 72%', once: true } });
      }
    }
    /* solutions: pin the section and move the rail sideways by exactly its overflow */
    mm.add('(min-width: 960px) and (prefers-reduced-motion: no-preference)', () => {
      const jobs = $('#jobs'), track = $('[data-jobs-track]'), bar = $('[data-jobs-bar]');
      jobs.classList.add('is-rail');
      /* the row starts on the content's left edge (first card whole, the next ones peeking in on the right),
         then scrolling pans it left until the last card meets the content's right edge */
      const cards = [...track.children], last = cards[cards.length - 1];
      const railLeft = () => track.parentElement.getBoundingClientRect().left;
      const xEnd = () => Math.min(0, $('#jobs .jobs-bar').getBoundingClientRect().right - railLeft() - (last.offsetLeft + last.offsetWidth));
      const pan = () => Math.max(1, -xEnd());
      const lead = () => Math.min(innerWidth * 0.24, 360);
      const jobsPin = ScrollTrigger.create({ trigger: jobs, start: 'top top', end: () => `+=${Math.round(Math.max(innerHeight * 0.6, pan() * 1.1))}`, pin: true, anticipatePin: 1, invalidateOnRefresh: true });
      /* two legs in scroll proportion: while the section arrives the row slides in from the right and settles on the
         left edge exactly as the pin starts; while pinned it pans to the end of the row */
      const approach = innerHeight * 0.85, pinned = Math.max(innerHeight * 0.6, pan() * 1.1);
      const tw = gsap.timeline({ scrollTrigger: { trigger: jobs, start: 'top 85%', end: () => jobsPin.end, scrub: 0.6, invalidateOnRefresh: true,
          onUpdate: (st) => { const lead0 = approach / (approach + pinned); bar.style.setProperty('--jp', (0.25 + 0.75 * clamp((st.progress - lead0) / (1 - lead0), 0, 1)).toFixed(3)); } } })
        .fromTo(track, { x: () => lead() }, { x: 0, ease: 'none', duration: approach })
        .to(track, { x: () => xEnd(), ease: 'none', duration: pinned });
      return () => { tw.scrollTrigger && tw.scrollTrigger.kill(); tw.kill(); jobsPin.kill(); gsap.set(track, { x: 0 }); jobs.classList.remove('is-rail'); };
    });
    mm.add('(max-width: 959px), (prefers-reduced-motion: reduce)', () => staticMode());
  } else {
    staticMode();
    initSort($('#pile'), { onChange: ({ done, wait }) => { const m = $('#pile .pile-empty'); if (m) m.textContent = `Morning. ${done} done${wait ? ', 1 waiting for your yes' : ''}.`; } });
  }

  /* day and night: the button flips the mode and remembers it; until then the page follows the system */
  const root = page, themeBtn = $('[data-theme-toggle]'), themeMeta = document.querySelector('meta[name="theme-color"]');
  if (themeMeta) { const before = themeMeta.getAttribute('content'); env.onStop(() => (before === null ? themeMeta.removeAttribute('content') : themeMeta.setAttribute('content', before))); }
  let savedTheme = null;
  try { savedTheme = localStorage.getItem('nyxid-theme'); } catch (e) { savedTheme = null; }
  const applyTheme = (t) => {
    root.dataset.theme = t;
    themeBtn.setAttribute('aria-label', t === 'dark' ? 'Switch to day mode' : 'Switch to night mode');
    if (themeMeta) themeMeta.setAttribute('content', t === 'dark' ? '#0E0E10' : '#F3F3F0');
    const end = $('#end');
    if (end) end.classList.toggle('on-ink', t === 'dark');
    /* the nav picks its glass from what is under it; ask it again now the mode has changed */
    requestAnimationFrame(() => dispatchEvent(new Event('scroll')));
  };
  applyTheme(root.dataset.theme === 'dark' ? 'dark' : 'light');
  themeBtn.addEventListener('click', () => {
    const next = root.dataset.theme === 'dark' ? 'light' : 'dark';
    try { localStorage.setItem('nyxid-theme', next); savedTheme = next; } catch (e) { /* not saved; the switch still applies to this visit */ }
    /* during the crossfade the page is hidden from hit-testing, so the nav asks again once it has finished */
    if (document.startViewTransition && !reduce) document.startViewTransition(() => applyTheme(next)).finished.then(() => dispatchEvent(new Event('scroll')));
    else applyTheme(next);
  });
  /* someone who has not chosen keeps following the system as it changes (for example at sunset) */
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (e) => { if (!savedTheme) applyTheme(e.matches ? 'dark' : 'light'); }, { signal: env.signal });

  /* nav: dark glass while it sits over an ink section */
  /* position bars: the section in the middle of the screen is marked, with the page's progress below */
  const hud = $('[data-hud]');
  if (hud) {
    const links = $$('a', hud), hudY = $('[data-hud-y]');
    const targets = links.map((a) => $(a.getAttribute('href')));
    let tick = false;
    const update = () => {
      tick = false;
      const mid = innerHeight * 0.5;
      let here = 0;
      targets.forEach((t, i) => { if (t && t.getBoundingClientRect().top <= mid) here = i; });
      links.forEach((a, i) => a.classList.toggle('is-here', i === here));
      const max = document.documentElement.scrollHeight - innerHeight;
      setText(hudY, String(Math.round(clamp(scrollY / (max || 1), 0, 1) * 100)).padStart(3, '0'));
    };
    addEventListener('scroll', () => { if (!tick) { tick = true; requestAnimationFrame(update); } }, { passive: true });
    update();
  }

  const nav = $('[data-nav]');
  /* ask what is actually painted under the nav: an ink section can sit hidden under the next section's rounded top */
  const tone = () => { const hit = document.elementsFromPoint(innerWidth / 2, 40).find((n) => !n.closest('.nav')); nav.classList.toggle('is-dark', !!(hit && hit.closest('.on-ink'))); };
  addEventListener('scroll', tone, { passive: true }); tone();

  /* ---------- 5. apps hub: lines from every chat to NyxID and out to every app ---------- */
  const hub = $('[data-hub]');
  function drawHub() {
    const svg = $('.hub-lines', hub);
    if (getComputedStyle(svg).display === 'none') return;
    const r = hub.getBoundingClientRect();
    const core = $('[data-core]', hub).getBoundingClientRect();
    const cy = core.top - r.top + core.height / 2;
    const cL = core.left - r.left, cR = core.right - r.left;
    svg.setAttribute('viewBox', `0 0 ${r.width} ${r.height}`);
    const paths = [];
    const add = (d, i, pulse) => {
      paths.push(svgPath({ 'data-i': i, d }));
      if (pulse) paths.push(svgPath({ class: 'pulse', d, style: `animation-delay:${(-(i * 0.83) % 3.2).toFixed(2)}s` }));
    };
    const tiles = $$('.tile', hub);
    const outs = tiles.filter((t) => t.dataset.node === 'out');
    const minLeft = Math.min(...outs.map((t) => t.getBoundingClientRect().left));
    tiles.forEach((t, i) => {
      const b = t.getBoundingClientRect();
      const y = b.top - r.top + b.height / 2;
      if (t.dataset.node === 'in') {
        const x = b.right - r.left, mx = (x + cL) / 2;
        add(`M${x} ${y} C${mx} ${y} ${mx} ${cy} ${cL} ${cy}`, i, true);
      } else if (b.left - minLeft < 4) {
        const x = b.left - r.left, mx = (x + cR) / 2;
        add(`M${cR} ${cy} C${mx} ${cy} ${mx} ${y} ${x} ${y}`, i, true);
      } else {
        const x = b.left - r.left;
        add(`M${x - 10} ${y} L${x} ${y}`, i, false);
      }
    });
    svg.replaceChildren(...paths);
    tiles.forEach((t, i) => {
      const path = $(`path[data-i="${i}"]`, svg);
      t.onmouseenter = () => path && path.classList.add('is-hot');
      t.onmouseleave = () => path && path.classList.remove('is-hot');
    });
  }
  drawHub();
  let hubT;
  addEventListener('resize', () => { clearTimeout(hubT); hubT = setTimeout(drawHub, 150); });

  /* ---------- 6. setup command ---------- */
  const TERM = {
    claude: () => [make('s', null, '$'), ' claude mcp add --transport http --scope user ', make('i', null, 'nyxid'), ' http://localhost:3001/mcp'],
    cursor: () => [make('s', null, '$'), ' nyxid mcp config --tool ', make('i', null, 'cursor')],
  };
  const termOut = $('[data-term-out]');
  $$('[data-term]').forEach((tab) => tab.addEventListener('click', () => {
    $$('[data-term]').forEach((t) => t.setAttribute('aria-selected', String(t === tab)));
    termOut.replaceChildren(...TERM[tab.dataset.term]());
  }));
  $('[data-copy]').addEventListener('click', (e) => {
    const text = termOut.textContent.replace(/^\$\s*/, '');
    const btn = e.currentTarget;
    const ok = () => { btn.textContent = 'Copied'; setTimeout(() => { btn.textContent = 'Copy'; }, 1600); };
    if (navigator.clipboard) navigator.clipboard.writeText(text).then(ok, ok); else ok();
  });

  /* ---------- 6. your agent: one request, followed card by card ----------
     0 the agent sends its own key, 1 it travels, 2 NyxID checks and swaps the key, 3 it travels on, 4 Stripe acts. Then again. */
  const flow = $('[data-flow]');
  if (flow) {
    const cards = $$('[data-flow-at]', flow);
    const HOLD = [800, 450, 1400, 450, 1900];
    let step = 4, flowT = 0;
    const showStep = (n) => {
      step = n;
      flow.dataset.step = n;
      for (let i = 1; i <= 4; i++) flow.classList.toggle(`s${i}`, n >= i);
      cards.forEach((c) => c.classList.toggle('is-on', +c.dataset.flowAt === n));
    };
    const nextStep = () => { showStep((step + 1) % 5); flowT = setTimeout(nextStep, HOLD[step]); };
    showStep(4);
    if (!reduce && 'IntersectionObserver' in window) {
      new IntersectionObserver(([en]) => {
        clearTimeout(flowT);
        if (en.isIntersecting) { showStep(0); flowT = setTimeout(nextStep, HOLD[0]); }
      }, { threshold: 0.35 }).observe(flow);
    }
  }

  /* the jobs bar follows a swipe when the rail scrolls natively */
  const jobsRail = $('[data-jobs-rail]'), jobsBar = $('[data-jobs-bar]');
  jobsRail.addEventListener('scroll', () => {
    const max = jobsRail.scrollWidth - jobsRail.clientWidth;
    jobsBar.style.setProperty('--jp', (0.34 + 0.66 * (max > 0 ? jobsRail.scrollLeft / max : 1)).toFixed(3));
  }, { passive: true });

  /* ---------- 8. closing line ---------- */
  if (hasGsap && !reduce) {
    const when = { trigger: $('[data-close]'), start: 'top 62%', once: true };
    if (Split) {
      SplitText.create($('.big'), { type: 'lines', mask: 'lines', linesClass: 'ln', autoSplit: true,
        onSplit: (self) => gsap.from(self.lines, { yPercent: 108, duration: 1.1, ease: 'power4.out', stagger: 0.12, scrollTrigger: when }) });
    } else {
      gsap.from($('.big'), { y: 30, opacity: 0, duration: 1, ease: 'power4.out', scrollTrigger: when });
    }
    gsap.from($('.close-cta'), { y: 20, opacity: 0, duration: 0.7, ease: 'power3.out', delay: 0.35, scrollTrigger: when });
  }

  /* ---------- nav: links decode on hover (ScrambleText) ---------- */
  if (Scramble && !reduce) {
    $$('.nav-links a').forEach((a) => {
      const label = a.textContent;
      a.addEventListener('pointerenter', () => {
        a.style.width = `${a.offsetWidth}px`;
        gsap.to(a, { duration: 0.4, overwrite: true, scrambleText: { text: label, chars: 'lowerCase', speed: 0.9 } });
      });
    });
  }


  /* ---------- the close: Nyx in halftone, revealed by scroll ----------
     The whole section is one dot grid, about 2.3-4px depending on the screen. Inside Moreau's oval each dot takes the painting's colour, sized by its
     brightness; outside it the same grid carries one even, faint lilac dot, and the painting fades into that field
     over a wide band at its edge, so it dissolves into the background instead of sitting in a frame.
     Scrolling into the section develops the picture all over at once: every dot swells slowly from nothing to its full
     size while the whole canvas fades in. Each dot starts a moment apart from its neighbours and the top rows lead the
     bottom ones slightly, but the growth is so long next to those offsets that no edge or line ever forms.
     The reveal is eased toward the scroll position; scrolling back takes the dots away. The pointer swells the dots it passes over.
     The dots are drawn by the graphics card (WebGL): the page hands it one small table with every dot's colour, size
     and turn, and each frame it works out how far each dot has grown. Drawing 200,000 dots one by one in script took
     longer than a frame and made the page stutter. Without WebGL a plain canvas draws the dots once and the fade
     alone reveals them. */
  function nyxDots(canvas0) {
    let canvas = canvas0;
    const artBox = canvas.parentNode;
    const host = canvas.closest('footer, section') || canvas; /* the cursor lens listens on the whole section */
    const img = new Image();
    const LENS = 120;
    /* reveal timing: rows take their turn over SPAN, each dot starts up to SCATTER later, and grows in over GROW.
       GROW is three times SPAN, so the top and the bottom of the picture are never more than a third of a dot's growth
       apart: the picture strengthens everywhere together. (It was 0.9 / 0.06 / 0.1: a narrow front that swept down the screen.) */
    const SPAN = 0.2, SCATTER = 0.2, GROW = 0.6, END = SPAN + SCATTER + GROW;
    /* the dot size follows the painting's height, so Nyx gets about the same number of dots on a laptop as on a big monitor */
    let CELL = 4, W = 0, H = 0, dpr = 1, cols = 0, rows = 0, paper = false;
    /* one entry per dot: rad its full radius in px (0 = no dot), rgba its colour, turn its random place in the scatter */
    let rad = null, rgba = null, turn = null, pic = null; /* pic: 255 for a dot of the picture, 0 for a dot of the faint field around it */
    /* the desktop panel's right-hand fade, for the WebGL painter, which draws it live so it can drift (see the shader):
       where it ends on average, how wide it is, and how far its end drifts either way, in px. clock is the time, in seconds. */
    const fade = { end: 0, width: 0, drift: 0 };
    let clock = 0;
    let ready = false, want = 0, target = 0, revealRaf = 0, lens = null, lensRaf = 0, painter = null;

    function sample() {
      paper = page.dataset.theme === 'light';
      dpr = Math.min(devicePixelRatio || 1, 2);
      W = canvas.clientWidth; H = canvas.clientHeight;
      if (!W || !H) return false;
      canvas.width = Math.round(W * dpr); canvas.height = Math.round(H * dpr);
      /* the size is rounded to half a screen pixel: an uneven size makes neighbouring dots land differently on the
         pixel grid and shows as a woven pattern */
      /* extra: on a desktop the painting's box also covers the footer (--art-extra px), so the picture can blend down
         to the bottom of the page instead of being cut at the footer's top. Everything about her size and place is
         still worked out from the box without that extra (Hg, rowsG): the extra is room to fade into, no more. */
      const extra = Math.min(H * 0.5, parseFloat(getComputedStyle(host).getPropertyValue('--art-extra')) || 0);
      /* topX: on a desktop the box also starts higher, at the very top of the section, covering the empty lead-in
         above the old box (the section's top padding). That is room for the picture to carry on upward and blend into
         the section above. It is measured, not read from the stylesheet: how far the box's top is above where the
         padding ends. On a phone it is 0. As with `extra`, her size and place ignore it (Hg is the box without both). */
      const topX = Math.max(0, Math.min(H * 0.4, (parseFloat(getComputedStyle(host).paddingTop) || 0) - artBox.offsetTop));
      const Hg = H - extra - topX;
      CELL = Math.round(Math.max(2.3, Math.min(4, Hg / 375)) * dpr * 2) / 2 / dpr;
      cols = Math.ceil(W / CELL); rows = Math.ceil(H / CELL);
      const rowsG = Math.ceil(Hg / CELL);
      const off = document.createElement('canvas');
      const o = off.getContext('2d', { willReadFrequently: true });
      /* Moreau painted Nyx inside an oval set in a gilt frame. Only the oval is sampled (OVAL is its box in the
         source, as fractions); the frame never shows. */
      const OVAL = { x: 0.062, y: 0.05, w: 0.876, h: 0.905 };
      const sw = img.naturalWidth * OVAL.w, sh = img.naturalHeight * OVAL.h;
      /* the oval lives inside the page's content column, the same left and right lines the nav and the headline use,
         so it never runs out toward the screen edge. On a desktop it is sized from the column's left line
         (the nav logo's left edge), growing rightward toward the words, then slid a little left (see slide below);
         on a phone it is centred. */
      const cr = canvas.getBoundingClientRect(), wrapEl = host.querySelector('.wrap');
      let colL = 0, colR = W;
      if (wrapEl) {
        const wr = wrapEl.getBoundingClientRect(), ws = getComputedStyle(wrapEl);
        colL = wr.left + (parseFloat(ws.paddingLeft) || 0) - cr.left; colR = wr.right - (parseFloat(ws.paddingRight) || 0) - cr.left;
      }
      const wideArt = W > 900;
      const boxX = colL / CELL, boxW = Math.max(1, (wideArt ? Math.min(W * 0.5, colR) : colR) / CELL - boxX);
      /* the base size fills the column's left half (or is a little taller than the screen, whichever is smaller).
         On a desktop the painting is drawn 30% larger than that, growing rightward from the column's left line, with two limits:
         - at most a fifth taller than the screen, or the star crown and the feet are both cut off;
         - it shrinks back until its visible edge clears the closing headline by 16px.
         On a phone or tablet it keeps the base size: the column is the whole screen there, so anything larger would run
         into the screen edges and into the headline below it. */
      const base = Math.min(boxW / sw, (rowsG * 1.06) / sh);
      let sc = wideArt ? Math.min(base * 1.3, (rowsG * 1.2) / sh) : base;
      /* SLIDE: how far the oval is slid left on a desktop, as a share of its own width. EDGE: the room, in px, its box keeps from the screen's left edge. */
      const SLIDE = 0.24, EDGE = 0;
      /* guardX: the x, in px, that no dot of the painting may pass on a desktop (16px short of the words) */
      let guardX = Infinity;
      const head = host.querySelector('.big');
      if (wideArt && head) {
        const hb = head.getBoundingClientRect(), rg = document.createRange();
        rg.selectNodeContents(head);
        /* the words are on the right. Two limits on the painting's width, both with a 16px gap:
           - fit: the painting (before it is slid) and the words' block fit side by side inside the content column.
             This is the rule that sets the size on a normal desktop, and it does not depend on where the words are placed;
           - actual: the painting where it really sits (slid left) stops short of where the text really starts.
             This one only bites on a narrow desktop, where the painting has no room to slide. */
        let textL = hb.right;
        for (const q of rg.getClientRects()) if (q.width > 0) textL = Math.min(textL, q.left);
        const cta = host.querySelector('.close-cta');
        const blockW = Math.max(hb.width, cta ? cta.getBoundingClientRect().width : 0);
        const needFit = colR - blockW - (hb.left - textL) - 16, needAct = textL - cr.left - 16;
        const hTop = hb.top - cr.top, hBot = hb.bottom - cr.top;
        guardX = needAct;
        for (let k = 0; k < 30 && sc > base; k++) {
          const pw = sw * sc * CELL, ph = sh * sc * CELL, cx = colL + pw / 2, cy = topX + (Hg - ph) / 2 + Hg * 0.03 + ph / 2;
          /* the oval is widest at its own middle: test it at the headline's line nearest to that */
          const v = (Math.min(hBot, Math.max(hTop, cy)) - cy) / (ph / 2);
          if (Math.abs(v) >= 1) break;
          /* 0.95: the outer 5% of the oval is faded to nothing */
          const reach = 0.95 * (pw / 2) * Math.sqrt(1 - v * v), slid = Math.min(pw * SLIDE, Math.max(0, colL - EDGE));
          if (cx + reach <= needFit && cx - slid + reach <= needAct) break;
          sc = Math.max(base, sc * 0.98);
        }
      }
      const dw = sw * sc, dh = sh * sc;
      /* centred top to bottom, sitting slightly low so the star crown clears the nav; what is taller than the screen falls outside, in the faded rim */
      /* desktop: the oval is slid left by SLIDE (24%) of its own width. The first 14% put the goddess's raised arm (about
         17% into the oval) under the nav's logo, where before only the oval's dark, faded rim sat; the next 10% move her
         further left again, so the arm now sits to the left of the logo. The slide stops where the oval's box would
         pass the screen's left edge (EDGE px inside it), so the oval is never cut off. */
      const slide = wideArt ? Math.min(dw * SLIDE, Math.max(0, boxX - EDGE / CELL)) : 0;
      const ox = wideArt ? boxX - slide : (W / CELL - dw) / 2, oy = topX / CELL + (rowsG - dh) / 2 + rowsG * 0.03;
      /* --- the painting's pixels, and its extension past the oval ---
         The source only has paint inside the oval; outside it is the gilt frame. To let the picture run on past the
         oval, the area outside is filled with the painting's own edge, mirrored: a point a little outside the rim takes
         the colour of the point the same distance inside it. Further out the mirrored picture is swapped for a heavily
         blurred copy of itself, so the extension carries the painting's colours and light but no second star or wing.
         1. `sharp`: the oval at one pixel per dot.
         2. `soft`: the same, with the frame replaced by the mirror, then blurred.
         3. `data`: for every dot of the whole section, its colour from those two. */
      const E0 = 0.94; /* the mirror line, as a share of the oval's radius: just inside the rim, clear of the frame's shadow */
      const aw = Math.max(2, Math.round(dw)), ah = Math.max(2, Math.round(dh));
      off.width = aw; off.height = ah;
      o.drawImage(img, img.naturalWidth * OVAL.x, img.naturalHeight * OVAL.y, sw, sh, 0, 0, aw, ah);
      let sharp, soft, amb;
      try { sharp = o.getImageData(0, 0, aw, ah); } catch (e) { return false; /* a file:// page cannot read pixels */ }
      const A = sharp.data, at = (v, m) => Math.min(m - 1, Math.max(0, Math.floor(v)));
      {
        const filled = new ImageData(new Uint8ClampedArray(A), aw, ah), F = filled.data;
        for (let y = 0; y < ah; y++) for (let x = 0; x < aw; x++) {
          const u = (x + 0.5) / aw - 0.5, v = (y + 0.5) / ah - 0.5, r = 2 * Math.sqrt(u * u + v * v);
          if (r <= E0) continue;
          const k = Math.max(0.05, 2 * E0 - r) / r, d = (y * aw + x) * 4, s = (at((v * k + 0.5) * ah, ah) * aw + at((u * k + 0.5) * aw, aw)) * 4;
          F[d] = A[s]; F[d + 1] = A[s + 1]; F[d + 2] = A[s + 2]; F[d + 3] = 255;
        }
        /* blur: shrink to a twelfth and stretch back */
        const small = document.createElement('canvas'), sm = small.getContext('2d');
        small.width = Math.max(2, Math.round(aw / 12)); small.height = Math.max(2, Math.round(ah / 12));
        o.putImageData(filled, 0, 0);
        sm.imageSmoothingEnabled = true; sm.imageSmoothingQuality = 'high';
        sm.drawImage(off, 0, 0, small.width, small.height);
        o.imageSmoothingEnabled = true; o.imageSmoothingQuality = 'high';
        o.drawImage(small, 0, 0, small.width, small.height, 0, 0, aw, ah);
        soft = o.getImageData(0, 0, aw, ah).data;
        /* a second, far stronger blur (a fortieth): only the painting's broad colour and light are left, no shapes.
           It fills the corners of the panel, which lie well outside the oval. */
        small.width = Math.max(2, Math.round(aw / 40)); small.height = Math.max(2, Math.round(ah / 40));
        sm.imageSmoothingEnabled = true; sm.imageSmoothingQuality = 'high';
        sm.drawImage(off, 0, 0, small.width, small.height);
        o.drawImage(small, 0, 0, small.width, small.height, 0, 0, aw, ah);
        amb = o.getImageData(0, 0, aw, ah).data;
      }
      /* a dot outside the oval looks back inside it for its colour: the further out it is, the further in it looks, but
         only a third as far (so the corners of the panel take the sky near the rim, never her body in the middle).
         Just past the rim it reads the sharp picture, then the soft one, and from a little further out, gradually, the far blur. */
      const data = new Uint8ClampedArray(cols * rows * 4);
      for (let i = 0, m = cols * rows; i < m; i++) {
        let u = ((i % cols) + 0.5 - ox) / dw - 0.5, v = (Math.floor(i / cols) + 0.5 - oy) / dh - 0.5, w = 0, w2 = 0;
        const r = 2 * Math.sqrt(u * u + v * v);
        if (r > E0) { const k = Math.max(0.5, E0 - (r - E0) * 0.35) / r; u *= k; v *= k; w = Math.min(1, (r - E0) / 0.14); w2 = Math.min(1, Math.max(0, (r - 1.1) / 0.45)); }
        const s = (at((v + 0.5) * ah, ah) * aw + at((u + 0.5) * aw, aw)) * 4;
        for (let k = 0; k < 3; k++) { const near = A[s + k] + (soft[s + k] - A[s + k]) * w; data[i * 4 + k] = near + (amb[s + k] - near) * w2; }
      }
      /* the extension's outline: a smooth random field (three layers of value noise, the same on every load, so day and
         night and every resize draw the same shape). It decides, direction by direction, how far past the oval the
         picture reaches, which makes the outline an uneven cloud and not a second, larger oval. */
      const hash = (x, y) => { let h = (Math.imul(x, 374761393) + Math.imul(y, 668265263)) | 0; h = Math.imul(h ^ (h >>> 13), 1274126177); return ((h ^ (h >>> 16)) >>> 0) / 4294967295; };
      const vnoise = (x, y) => {
        const xi = Math.floor(x), yi = Math.floor(y), fx = x - xi, fy = y - yi, ex = fx * fx * (3 - 2 * fx), ey = fy * fy * (3 - 2 * fy);
        const a = hash(xi, yi), b = hash(xi + 1, yi), c = hash(xi, yi + 1), d = hash(xi + 1, yi + 1);
        return a + (b - a) * ex + (c - a) * ey + (a - b - c + d) * ex * ey;
      };
      const cloud = (x, y) => (vnoise(x, y) + 0.5 * vnoise(x * 2.1 + 11.5, y * 2.1 + 4.2) + 0.25 * vnoise(x * 4.3 + 3.1, y * 4.3 + 8.7)) / 1.75;
      /* desktop panel: fillEnd is where its right-hand fade has reached nothing (16px short of the words, or 55% of the
         screen if the words cannot be measured); BLEND is how wide that fade is */
      const tall = dh / dw, fillEnd = isFinite(guardX) ? guardX : W * 0.55, BLEND = clamp(W * 0.15, 130, 260);
      /* with WebGL the fade is drawn live and its end drifts by up to DRIFT either way (never past fillEnd); the plain
         canvas cannot redraw every frame, so there the fade is worked into the dots once and stands still */
      const bakeFade = !painter || painter.soft, DRIFT = reduce ? 0 : BLEND * 0.18;
      fade.end = fillEnd - DRIFT; fade.width = wideArt && !bakeFade ? BLEND : 0; fade.drift = DRIFT;
      /* tell the stylesheet where the oval is, for the night halo behind it (a background, see .close-art canvas):
         its centre, and 1.3 times its half-width and half-height. Written on the painting's box, so a replaced canvas still reads it. */
      artBox.style.setProperty('--hx', `${((ox + dw / 2) * CELL).toFixed(1)}px`);
      artBox.style.setProperty('--hy', `${((oy + dh / 2) * CELL).toFixed(1)}px`);
      /* a desktop has the filled panel and no oval halo (its size is set to nothing); a phone has the halo and no panel */
      artBox.style.setProperty('--hw', wideArt ? '0px' : `${((dw / 2) * CELL * 1.3).toFixed(1)}px`);
      artBox.style.setProperty('--hh', wideArt ? '0px' : `${((dh / 2) * CELL * 1.3).toFixed(1)}px`);
      /* the panel's two straight edges, for the stylesheet (see .close-art canvas): the right-hand fade starts at --fs
         and is --fw wide; the bottom fade runs from --bs (the top of the footer) to --be (the bottom of
         the page). The picture itself is not faded: it carries on down through the footer, and the footer is where it
         blends out. (The top works the same way, upward into the lead-in above the picture; see .close-art.) */
      /* (the night ground's fade is more than twice as wide as the dots' and carries on behind the start of the words:
         it is only a faint tint, and the longer it takes to go, the less there is a place where the page seems to split) */
      for (const [k, val] of [['--fs', fillEnd - BLEND * 1.1], ['--fw', BLEND * 2], ['--bs', topX + Hg - 12], ['--be', H - 2]]) {
        if (wideArt) artBox.style.setProperty(k, `${val.toFixed(1)}px`); else artBox.style.removeProperty(k);
      }
      const n = cols * rows;
      /* day: the paper's colour (the section's light-mode background), used to work out the ink below */
      const paperC = [228, 226, 219];
      if (paper) { const hx = /^#([0-9a-f]{6})$/i.exec(getComputedStyle(host).getPropertyValue('--close-bg').trim()); if (hx) { const q = parseInt(hx[1], 16); paperC[0] = q >> 16; paperC[1] = (q >> 8) & 255; paperC[2] = q & 255; } }
      rad = new Float32Array(n); rgba = new Uint8Array(n * 4); turn = new Uint8Array(n); pic = new Uint8Array(n);
      let lo = 0, hi = 1;
      if (paper) {
        const inside = [];
        for (let i = 0; i < n; i++) {
          const u = ((i % cols) + 0.5 - ox) / dw - 0.5, v = (Math.floor(i / cols) + 0.5 - oy) / dh - 0.5;
          if ((u * u + v * v) * 4 < 0.7) inside.push((0.299 * data[i * 4] + 0.587 * data[i * 4 + 1] + 0.114 * data[i * 4 + 2]) / 255);
        }
        inside.sort((a, b) => a - b);
        lo = inside[Math.floor(inside.length * 0.05)] || 0; hi = inside[Math.floor(inside.length * 0.97)] || 1;
      }
      /* the field: one even, faint dot everywhere, so the right half reads the same as the left */
      const bgR = (CELL / 2) * (paper ? 0.18 : 0.22), bgA = paper ? 0.06 : 0.11;
      const field = paper ? [86, 80, 54] : [196, 182, 252];
      const tiny = 0.1 * CELL; /* a dot smaller than this is too small to see and is left out */
      for (let i = 0; i < n; i++) {
        const c = i % cols, r = Math.floor(i / cols);
        const u = (c + 0.5 - ox) / dw - 0.5, v = (r + 0.5 - oy) / dh - 0.5;
        const e = (u * u + v * v) * 4; /* 1 on the oval's edge */
        /* the painting, fading into the field over a wide band: dots shrink and dim toward the oval's edge */
        const R = data[i * 4], G = data[i * 4 + 1], B = data[i * 4 + 2];
        const lum = (0.299 * R + 0.587 * G + 0.114 * B) / 255;
        /* how much of the picture's ink this dot carries (s): 1 = all of it, 0 = none. */
        const re = Math.sqrt(e), xpx = (c + 0.5) * CELL, ypx = (r + 0.5) * CELL;
        let s = 0, seeThrough = 1;
        if (wideArt) {
          /* desktop: a filled panel. The picture is solid from the screen's left edge, top to bottom: inside the oval
             it is the painting, around it the painting's own blurred colours (see `data` above). There is no oval
             edge any more. Its right side fades toward the words over BLEND px, ending at fillEnd: a straight
             vertical line, the same at every height. Its bottom fades through the footer on a straight horizontal
             line; the stylesheet does that one (the canvas's mask), so dots and ground fade there together. */
          s = xpx < fillEnd ? 1 : 0;
          /* without WebGL the right-hand fade is worked into each dot's transparency here, once (seeThrough) */
          if (bakeFade && s) { const f = clamp((fillEnd - xpx) / BLEND, 0, 1); seeThrough = f * f * f * (f * (f * 6 - 15) + 10); if (seeThrough <= 0) s = 0; }
        } else if (re <= 0.6) s = 1;
        else if (re < 1.72) {
          /* phone and tablet: the picture sits above the words and fades outward in every direction. It is all there
             out to about three quarters of the oval's radius, then falls in one even slope through the rim and on to
             `reach` (about 1.3 to 1.6 of the radius, as the cloud field says for this direction); at the screen's
             side edges the fade just runs off the screen. */
          const reach = 1.15 + 0.55 * cloud(u * 3.1 + 7.3, v * 3.1 * tall + 2.9);
          if (re < reach) s = Math.min(1, 1.25 * (reach - re) / Math.max(0.05, reach - 0.6));
        }
        /* the slope is eased at both ends (s*s*(3-2s)): it leaves full strength gradually, and it arrives at nothing
           gradually. Without the easing the fade stopped while its dots were still a visible size, and that last step
           drew a faint outline around the whole picture. */
        s = s * s * (3 - 2 * s);
        /* a dot's ink is its area, and area goes with the square of its width: so for the ink to follow s the width
           must follow the square root of s (edge). Half the ink is a dot 71% as wide, not 50%. */
        const edge = Math.sqrt(s);
        /* day: stretch the tones, then size by ink so the lit figure is near paper and the night sky is full dots */
        let tone = lum;
        if (paper) { const s01 = Math.min(1, Math.max(0, (lum - lo) / (hi - lo || 1))); tone = 1 - s01 * s01 * (3 - 2 * s01); }
        /* day: lit areas keep smaller dots (so her gold skin still shows), dark areas grow to full dots.
           DAY_MIN is the smallest dot, for the brightest spot of the painting, as a share of the grid square's half-width.
           It was 0.42: the lit figure was then only about 14% ink and 86% bare paper, and read as white. At 0.64 it is
           about 32% ink, so her skin shows its gold before the pointer is anywhere near. The largest dot is unchanged
           (DAY_MIN + DAY_RANGE is still 1.26), so the dark sky looks as it did. Night mode has its own rule, after the colon. */
        const DAY_MIN = 0.64, DAY_RANGE = 1.26 - DAY_MIN;
        const full = paper ? (CELL / 2) * (DAY_MIN + DAY_RANGE * tone) : (CELL / 2) * (0.16 + 0.9 * Math.pow(tone, 0.6));
        let rr, cr0, cg0, cb0, ca;
        if (edge > 0.02) {
          rr = bgR + (full - bgR) * edge;
          if (paper) {
            /* day: the ink is chosen for what the eye ends up seeing. A dot covers only part of its square and pale
               paper shows around it, and that paper washes the colour out: her lit skin is about a third ink and two
               thirds paper, so a gold dot read as cream. So:
               1. the colour to be seen (want*) is the painting's own, saturation lifted by SAT and a touch darker (DK);
               2. `cover` is the share of the square this dot fills;
               3. the ink is pushed past that colour, away from the paper, by 1/cover, so ink and paper mix back to it.
               COVER_MIN stops the push at about 2.2 times: beyond that a small dot would need an ink no colour can be,
               and her skin turns pink instead of gold. */
            const g = 0.299 * R + 0.587 * G + 0.114 * B, SAT = 1.4, DK = 0.92, COVER_MIN = 0.45;
            /* (the cover is taken from the dot at its full size: where the picture is fading out the dot is meant to be lighter) */
            const m = full / (CELL / 2), cover = Math.max(COVER_MIN, Math.min(1, 0.785 * m * m));
            const wantR = (g + (R - g) * SAT) * DK, wantG = (g + (G - g) * SAT) * DK, wantB = (g + (B - g) * SAT) * DK;
            cr0 = Math.min(255, Math.max(0, paperC[0] + (wantR - paperC[0]) / cover)) | 0;
            cg0 = Math.min(255, Math.max(0, paperC[1] + (wantG - paperC[1]) / cover)) | 0;
            cb0 = Math.min(255, Math.max(0, paperC[2] + (wantB - paperC[2]) / cover)) | 0;
            ca = 0.4 + 0.6 * s; /* the dots also thin a little as they shrink, so the far end of the fade melts into the paper */
          } else {
            const k = 1.3;
            cr0 = Math.min(255, R * k) | 0; cg0 = Math.min(255, G * k) | 0; cb0 = Math.min(255, B * k + 6) | 0;
            ca = 0.25 + 0.75 * s;
          }
        } else {
          rr = bgR; cr0 = field[0]; cg0 = field[1]; cb0 = field[2]; ca = bgA;
        }
        rad[i] = rr < tiny ? 0 : rr; pic[i] = edge > 0.02 ? 255 : 0;
        rgba[i * 4] = cr0; rgba[i * 4 + 1] = cg0; rgba[i * 4 + 2] = cb0; rgba[i * 4 + 3] = Math.round(ca * (pic[i] ? seeThrough : 1) * 255);
        /* a dot's turn comes with its row, plus this little random wait */
        turn[i] = (Math.random() * 256) | 0;
      }
      return true;
    }

    /* fade the whole canvas for reveal value v (0 -> END). With WebGL the dots also swell by themselves, so the fade is
       done by the reveal's halfway point and the growth carries the rest; the plain canvas has finished dots, so its
       fade runs the whole way. The fade is eased at both ends (t*t*(3-2t)), so it neither starts nor stops abruptly. */
    function setReveal(v) {
      const t = clamp(v / (painter && painter.soft ? END : END * 0.5), 0, 1);
      canvas.style.opacity = (t * t * (3 - 2 * t)).toFixed(3);
    }

    /* --- the WebGL painter: one rectangle over the canvas; for every pixel the shader finds its dot and the four
       beside it (a full dot is a little wider than its square), works out how far each has grown, and stacks them --- */
    const FRAG = `
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
uniform vec2 uRes, uGrid, uLens;
uniform float uCell, uWant, uLensR, uMaxR, uCap, uSpan, uScatter, uGrow, uTime;
uniform vec3 uFade; /* the panel's right-hand fade: x where it ends, its width, how far that end drifts (all in px; width 0 = no fade) */
uniform sampler2D uCol, uDat;
vec4 dotAt(vec2 cell, vec2 p){
  if (cell.x < 0. || cell.y < 0. || cell.x >= uGrid.x || cell.y >= uGrid.y) return vec4(0.);
  vec2 uv = (cell + .5) / uGrid;
  vec4 d = texture2D(uDat, uv);
  float rad = d.r * uMaxR;
  if (rad <= 0.) return vec4(0.);
  /* this dot's turn: its row, plus its own random wait; then it grows over uGrow */
  float g = clamp((uWant - (cell.y / uGrid.y) * uSpan - uScatter * d.g) / uGrow, 0., 1.);
  if (g <= 0.) return vec4(0.);
  vec2 c = (cell + .5) * uCell;
  /* the panel's right-hand fade, for the picture's dots only (d.b marks them). Its end is not a fixed line: three
     slow waves of different lengths and speeds run along it, so it drifts a little in and out, differently at each
     height, like the edge of mist.
     The dots fade by becoming see-through and keep nearly all of their size (they lose a tenth), which is how the
     bottom edge fades too. They used to fade by shrinking: near the end of the fade that left a scatter of tiny hard
     specks, which read as grit, and it did not match the bottom edge where the two meet in the corner.
     The curve (f*f*f*(f*(f*6-15)+10)) is flat at both ends, so the fade has no visible start and no visible end. */
  float fade = 1.;
  if (d.b > .5 && uFade.y > 0.) {
    float yy = c.y / uRes.y * 6.2832;
    float wob = .5 * sin(yy * .9 + uTime * .5) + .3 * sin(yy * 2.1 - uTime * .33 + 1.7) + .2 * sin(yy * 3.7 + uTime * .21 + 4.1);
    float f = clamp((uFade.x + wob * uFade.z - c.x) / uFade.y, 0., 1.);
    fade = f * f * f * (f * (f * 6. - 15.) + 10.);
    if (fade <= 0.) return vec4(0.);
  }
  float q = 1. - min(1., distance(c, uLens) / uLensR);
  float r = min(uCap, rad * (.9 + .1 * fade) * (1. + .75 * q * q) * g * g * (3. - 2. * g));
  /* soft edge one pixel wide; the divisor and the small-dot branch keep a dot's total ink equal to its true area */
  float dist = distance(p, c);
  float cov = r < .5 ? clamp(1. - dist, 0., 1.) * 3. * r * r : clamp(r + .5 - dist, 0., 1.) / (1. + 1. / (12. * r * r));
  vec4 k = texture2D(uCol, uv);
  float a = k.a * cov * fade;
  return vec4(k.rgb * a, a);
}
void main(){
  vec2 p = vec2(gl_FragCoord.x, uRes.y - gl_FragCoord.y);
  vec2 cell = floor(p / uCell);
  vec4 o = dotAt(cell + vec2(0., -1.), p), s;
  s = dotAt(cell + vec2(-1., 0.), p); o = s + o * (1. - s.a);
  s = dotAt(cell, p); o = s + o * (1. - s.a);
  s = dotAt(cell + vec2(1., 0.), p); o = s + o * (1. - s.a);
  s = dotAt(cell + vec2(0., 1.), p); o = s + o * (1. - s.a);
  gl_FragColor = o;
}`;
    let glTaken = false; /* a canvas that has given out a WebGL context cannot give a 2D one: the fallback needs a fresh canvas */
    function glPainter() {
      const gl = canvas.getContext('webgl', { alpha: true, premultipliedAlpha: true, antialias: false });
      if (!gl) return null;
      glTaken = true;
      const sh = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null; };
      const vs = sh(gl.VERTEX_SHADER, 'attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}'), fs = sh(gl.FRAGMENT_SHADER, FRAG);
      if (!vs || !fs) return null;
      const prog = gl.createProgram(); gl.attachShader(prog, vs); gl.attachShader(prog, fs); gl.linkProgram(prog);
      if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) return null;
      gl.useProgram(prog);
      gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
      const loc = gl.getAttribLocation(prog, 'p'); gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
      const U = {};
      for (const k of ['uRes', 'uGrid', 'uLens', 'uCell', 'uWant', 'uLensR', 'uMaxR', 'uCap', 'uSpan', 'uScatter', 'uGrow', 'uCol', 'uDat', 'uFade', 'uTime']) U[k] = gl.getUniformLocation(prog, k);
      /* two small tables, one entry per dot: colours, and size with turn */
      const table = (unit) => {
        gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      };
      table(0); table(1);
      gl.uniform1i(U.uCol, 0); gl.uniform1i(U.uDat, 1);
      gl.uniform1f(U.uSpan, SPAN); gl.uniform1f(U.uScatter, SCATTER); gl.uniform1f(U.uGrow, GROW);
      gl.clearColor(0, 0, 0, 0);
      let lost = false;
      /* a lost graphics context hands the picture to the plain canvas */
      canvas.addEventListener('webglcontextlost', (e) => { e.preventDefault(); lost = true; usePlainCanvas(); });
      const draw = () => {
        if (lost || !ready) return;
        gl.uniform1f(U.uWant, want); gl.uniform1f(U.uTime, clock);
        if (lens) gl.uniform2f(U.uLens, lens.x * dpr, lens.y * dpr); else gl.uniform2f(U.uLens, -1e6, -1e6);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      };
      return {
        soft: false,
        load() {
          const n = cols * rows, maxR = 0.64 * CELL, dat = new Uint8Array(n * 4);
          for (let i = 0; i < n; i++) { dat[i * 4] = Math.min(255, Math.round((rad[i] / maxR) * 255)); dat[i * 4 + 1] = turn[i]; dat[i * 4 + 2] = pic[i]; dat[i * 4 + 3] = 255; }
          gl.uniform3f(U.uFade, fade.end * dpr, fade.width * dpr, fade.drift * dpr);
          gl.viewport(0, 0, canvas.width, canvas.height);
          gl.activeTexture(gl.TEXTURE0); gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, cols, rows, 0, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
          gl.activeTexture(gl.TEXTURE1); gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, cols, rows, 0, gl.RGBA, gl.UNSIGNED_BYTE, dat);
          gl.uniform2f(U.uRes, canvas.width, canvas.height); gl.uniform2f(U.uGrid, cols, rows);
          gl.uniform1f(U.uCell, CELL * dpr); gl.uniform1f(U.uLensR, LENS * dpr);
          gl.uniform1f(U.uMaxR, maxR * dpr); gl.uniform1f(U.uCap, CELL * 0.62 * dpr);
          draw();
        },
        reveal: draw,
        lens: draw,
      };
    }

    /* --- the plain-canvas painter (no WebGL): every dot is drawn once, in slices of about 6ms so no frame is held up;
       the CSS mask does the reveal, and the lens redraws only the cells under it --- */
    function plainPainter() {
      const ctx = canvas.getContext('2d');
      if (!ctx) return null;
      let col = null, built = 0, job = 0, lastLens = null;
      const dot = (i, boost) => {
        if (!rad[i]) return;
        const x = ((i % cols) + 0.5) * CELL * dpr, y = (Math.floor(i / cols) + 0.5) * CELL * dpr;
        ctx.fillStyle = col[i];
        ctx.beginPath(); ctx.arc(x, y, Math.min(CELL * 0.62, rad[i] * boost) * dpr, 0, 6.2832); ctx.fill();
      };
      const boostAt = (i) => {
        if (!lens) return 1;
        const d = Math.hypot(((i % cols) + 0.5) * CELL - lens.x, (Math.floor(i / cols) + 0.5) * CELL - lens.y);
        return d < LENS ? 1 + 0.75 * (1 - d / LENS) ** 2 : 1;
      };
      /* redraw a block of cells (only the ones the drawing pass has reached, or a dot would be drawn twice) */
      const cells = (c0, r0, c1, r1) => {
        ctx.clearRect(c0 * CELL * dpr, r0 * CELL * dpr, (c1 - c0 + 1) * CELL * dpr, (r1 - r0 + 1) * CELL * dpr);
        for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) { const i = r * cols + c; if (i < built) dot(i, boostAt(i)); }
      };
      const box = (p) => [Math.max(0, Math.floor((p.x - LENS) / CELL)), Math.max(0, Math.floor((p.y - LENS) / CELL)), Math.min(cols - 1, Math.ceil((p.x + LENS) / CELL)), Math.min(rows - 1, Math.ceil((p.y + LENS) / CELL))];
      return {
        soft: true,
        load() {
          const n = cols * rows, mine = ++job;
          col = new Array(n);
          for (let i = 0; i < n; i++) if (rad[i]) col[i] = `rgba(${rgba[i * 4]},${rgba[i * 4 + 1]},${rgba[i * 4 + 2]},${(rgba[i * 4 + 3] / 255).toFixed(3)})`;
          ctx.clearRect(0, 0, canvas.width, canvas.height);
          built = 0; lastLens = null;
          const slice = () => {
            if (mine !== job) return; /* a newer pass (after a resize or a mode switch) retires this one */
            const stop = performance.now() + 6;
            while (built < n && performance.now() < stop) { const end = Math.min(n, built + 500); for (let i = built; i < end; i++) dot(i, 1); built = end; }
            if (built < n) requestAnimationFrame(slice);
            else if (lens) { cells(...box(lens)); lastLens = lens; }
          };
          slice();
        },
        reveal() {},
        lens() {
          /* the old and new lens usually overlap (the pointer moved a few pixels): redraw their shared block once */
          const a = lastLens && box(lastLens), b = lens && box(lens);
          if (a && b && a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]) cells(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[2], b[2]), Math.max(a[3], b[3]));
          else { if (a) cells(...a); if (b) cells(...b); }
          lastLens = lens;
        },
      };
    }
    /* switch to the plain canvas. The old canvas may already belong to WebGL, so it is swapped for a clean copy. */
    function usePlainCanvas() {
      if (glTaken) { const fresh = canvas.cloneNode(false); canvas.replaceWith(fresh); canvas = fresh; glTaken = false; }
      painter = plainPainter();
      redo();
    }

    /* measure, hand the dots to the painter and place the mask again: on load, after a resize, when the fonts
       arrive (they change the headline's width) and when day and night switch */
    function redo() {
      if (!painter || !img.complete || !img.naturalWidth) return;
      ready = sample();
      if (!ready) return;
      painter.load();
      setReveal(want);
    }
    function paintReveal() {
      revealRaf = 0;
      if (!ready) return;
      const d = target - want;
      want = Math.abs(d) < 0.002 ? target : want + d * 0.16;
      setReveal(want);
      painter.reveal();
      if (want !== target) revealRaf = requestAnimationFrame(paintReveal);
    }
    const reveal = (e) => { target = Math.min(END, Math.max(0, e)); if (!revealRaf) revealRaf = requestAnimationFrame(paintReveal); wake(); };
    /* the drift: while the section is on screen (and the tab is in front, and something of the picture is showing),
       the picture is redrawn about 30 times a second with the clock moved on, which is what makes the fade's edge
       drift. The loop stops by itself when any of those ends, and wake() starts it again. With reduced motion, or
       without WebGL, it never runs and the fade stands still. */
    let onScreen = false, driftRaf = 0, driftAt = 0;
    const drift = (now) => {
      driftRaf = 0;
      if (!onScreen || document.hidden || !ready || painter.soft || fade.width <= 0 || fade.drift <= 0 || (want <= 0 && target <= 0)) return;
      if (now - driftAt >= 32) { driftAt = now; clock = now / 1000; painter.reveal(); }
      driftRaf = requestAnimationFrame(drift);
    };
    function wake() { if (!driftRaf && !reduce) driftRaf = requestAnimationFrame(drift); }
    if ('IntersectionObserver' in window) new IntersectionObserver((es) => { onScreen = es[es.length - 1].isIntersecting; wake(); }).observe(artBox);
    else onScreen = true;
    document.addEventListener('visibilitychange', wake, { signal: env.signal });

    try { painter = glPainter(); } catch (e) { painter = null; }
    if (!painter) {
      if (glTaken) { const fresh = canvas.cloneNode(false); canvas.replaceWith(fresh); canvas = fresh; glTaken = false; }
      painter = plainPainter();
    }
    if (!painter) return;
    const scrollDriven = !reduce;
    img.addEventListener('load', () => {
      if (!env.alive) return;
      /* without scroll-driven motion the whole picture simply shows */
      if (!scrollDriven) want = target = END;
      redo();
      if (!ready || !scrollDriven) return;
      /* from the painting's top at 90% of the screen until the painting fills the screen exactly: the picture
         develops the whole way and is finished as she comes fully into view, before the footer */
      /* (the box now also covers the footer, so "the painting fills the screen" is that much before the box's own
         bottom reaches the bottom of the screen: the end is moved up by the footer's share, --art-extra) */
      const footShare = () => parseFloat(getComputedStyle(host).getPropertyValue('--art-extra')) || 0;
      env.ctx.add(() => ScrollTrigger.create({ trigger: artBox, start: 'top 90%', end: () => `bottom bottom+=${footShare()}`, invalidateOnRefresh: true, onUpdate: (st) => reveal(st.progress * END), onRefresh: (st) => reveal(st.progress * END) }));
    });
    img.src = canvas.dataset.nyx;
    /* switching day and night redraws the dots for the new ground */
    new MutationObserver(redo).observe(page, { attributes: true, attributeFilter: ['data-theme'] });
    let rt;
    addEventListener('resize', () => { clearTimeout(rt); rt = setTimeout(redo, 150); });
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(redo);
    if (reduce) return;
    const paintLens = () => { lensRaf = 0; if (ready) painter.lens(); };
    host.addEventListener('pointermove', (ev) => {
      const r = canvas.getBoundingClientRect();
      const x = ev.clientX - r.left, y = ev.clientY - r.top;
      lens = x >= -LENS && y >= -LENS && x <= W + LENS && y <= H + LENS ? { x, y } : null;
      if (!lensRaf) lensRaf = requestAnimationFrame(paintLens);
    }, { passive: true });
    host.addEventListener('pointerleave', () => { lens = null; if (!lensRaf) lensRaf = requestAnimationFrame(paintLens); });
  }
  $$('canvas[data-nyx]').forEach(nyxDots);

  /* fonts change every measured height */
  const settle = () => {
    if (!env.alive) return;
    drawHub(); aim(); placeAll();
    if (hasGsap) ScrollTrigger.refresh();
  };
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(settle);
  addEventListener('load', settle);

  }, page);

  env.onStop(() => {
    env.ctx.revert();
    page.replaceChildren(...pristine);
    ScrollTrigger.refresh();
  });
  return () => env.stop();
}
