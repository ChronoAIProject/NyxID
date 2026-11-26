/* Shared behaviour for the three combined NyxID mockups.
   - mounts the paper-slip sorting scene (ported from landing-2 v2) and the apps hub (ported from landing-1 v3)
   - GSAP Flip moves each slip into its tray, Draggable + InertiaPlugin let you throw one in yourself
   - Lenis smooths the scroll and drives ScrollTrigger
   Each page calls NyxKit.start({...}) and adds its own background animation through the hooks. */
(() => {
  const G = window.gsap;
  const has = !!G;
  const P = { ST: window.ScrollTrigger, Flip: window.Flip, Draggable: window.Draggable, Inertia: window.InertiaPlugin, Draw: window.DrawSVGPlugin, Motion: window.MotionPathPlugin };
  if (has) G.registerPlugin(...Object.values(P).filter(Boolean));
  document.documentElement.classList.add('js');

  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
  const setText = (el, t) => { if (el && el.textContent !== String(t)) el.textContent = t; };

  /* ---------- markup ---------- */
  const STAMP_OK = '<svg class="stamp" viewBox="0 0 40 40" aria-hidden="true"><circle cx="20" cy="20" r="17"/><path d="M12 20.5l5.5 5.5L28.5 14"/></svg>';
  const STAMP_WAIT = '<svg class="stamp" viewBox="0 0 40 40" aria-hidden="true"><circle cx="20" cy="20" r="17"/><path d="M20 11v11M20 28v.5"/></svg>';
  const PILE_HTML = `
  <div class="wrap pile-grid">
    <div class="pile-copy">
      <h2>The pile on your counter, sorted before you wake.</h2>
      <p class="lede">Seven things landed while Luna Bistro was closed. Scroll to watch NyxBot work through them, or pick one up and hand it over yourself.</p>
      <p class="count" aria-live="polite"><span><b data-n-done>0</b>done</span><span><b data-n-wait>0</b>waiting for your yes</span></p>
      <div class="pile" data-pile aria-label="Things on the counter">
        <p class="pile-empty">The counter is clear.</p>
        <button class="obj" type="button" data-obj="slip" data-dest="done" style="--x:2%;--y:4%;--r:-7deg;--z:2" aria-label="Reservation from Sarah. Hand to NyxBot.">
          <span class="obj-in"><span class="shape s-slip"><span class="o-l">Reservation</span><span class="o-m">Table for 4</span><span class="o-s">Sun 7:30 PM, Sarah</span></span>${STAMP_OK}</span></button>
        <button class="obj" type="button" data-obj="letter" data-dest="done" style="--x:45%;--y:0%;--r:5deg;--z:1" aria-label="Question on X from maya_eats. Hand to NyxBot.">
          <span class="obj-in"><span class="shape s-letter"><svg viewBox="0 0 100 66" preserveAspectRatio="none" aria-hidden="true"><path d="M0 0 L50 34 L100 0"/></svg><span class="o-l"><img src="assets/icons/x.svg" alt="">@maya_eats</span><span class="o-m">Gluten-free dishes?</span><span class="o-s">Open on Monday?</span></span>${STAMP_OK}</span></button>
        <button class="obj" type="button" data-obj="invoice" data-dest="wait" style="--x:68%;--y:30%;--r:8deg;--z:3" aria-label="Invoice from Harbour Fish Co. Hand to NyxBot.">
          <span class="obj-in"><svg class="clip" viewBox="0 0 12 26" aria-hidden="true"><path d="M3 8v11a3 3 0 006 0V5a2 2 0 00-4 0v12"/></svg><span class="shape s-invoice"><span class="o-l">Harbour Fish Co.</span><span class="o-m">Friday order</span><span class="o-s">Price up 8%. Confirm?</span></span>${STAMP_WAIT}</span></button>
        <button class="obj" type="button" data-obj="receipt" data-dest="done" style="--x:49%;--y:47%;--r:-9deg;--z:2" aria-label="Till receipt for Saturday. Hand to NyxBot.">
          <span class="obj-in"><span class="shape s-receipt"><span class="o-l">Saturday till</span><span class="row"><span>Tables</span><b>38</b></span><span class="row"><span>Card</span><b>31</b></span><span class="row"><span>Cash</span><b>7</b></span></span>${STAMP_OK}</span></button>
        <button class="obj" type="button" data-obj="sticky" data-dest="done" style="--x:27%;--y:25%;--r:-3deg;--z:4" aria-label="Note: Jo is sick, can't do lunch. Hand to NyxBot.">
          <span class="obj-in"><span class="shape s-sticky"><span class="o-m">Jo is sick, can't do lunch</span><span class="o-s">7:15 AM</span></span>${STAMP_OK}</span></button>
        <button class="obj" type="button" data-obj="call" data-dest="done" style="--x:0%;--y:58%;--r:4deg;--z:3" aria-label="Missed call from Mr Tan. Hand to NyxBot.">
          <span class="obj-in"><span class="shape s-call"><span class="o-l">Missed call</span><span class="o-m">Mr Tan</span><span class="o-s">6:05 PM, about Friday</span></span>${STAMP_OK}</span></button>
        <button class="obj" type="button" data-obj="cal" data-dest="done" style="--x:27%;--y:62%;--r:6deg;--z:5" aria-label="Morning brief, weekdays at 8. Hand to NyxBot.">
          <span class="obj-in"><span class="shape s-cal"><span class="cal-top">Weekdays</span><span class="cal-big">8:00</span><span class="o-s">Morning brief</span></span>${STAMP_OK}</span></button>
      </div>
      <p class="hint">Drag a slip into a tray, or press Enter on it.</p>
    </div>
    <div class="trays" data-trays>
      <div class="tray" data-tray="done">
        <p class="tray-h">Done by NyxBot <span data-h-done>nothing yet</span></p>
        <ol>
          <li class="slot" data-slot="slip"><span class="slot-hole"></span><p class="slot-text"><b>Sarah's table.</b> Booked for Sunday 7:30 PM, one vegan noted. Reply sent.</p><img src="assets/icons/telegram.svg" alt="Telegram"></li>
          <li class="slot" data-slot="letter"><span class="slot-hole"></span><p class="slot-text"><b>A question on X.</b> Answered from your menu notes.</p><img src="assets/icons/x.svg" alt="X"></li>
          <li class="slot" data-slot="receipt"><span class="slot-hole"></span><p class="slot-text"><b>Saturday's takings.</b> Added to your sheet, row 212.</p><img src="assets/icons/google-sheets.svg" alt="Google Sheets"></li>
          <li class="slot" data-slot="sticky"><span class="slot-hole"></span><p class="slot-text"><b>Jo is sick.</b> Sam said yes to lunch. Rota updated.</p><img src="assets/icons/google-calendar.svg" alt="Google Calendar"></li>
          <li class="slot" data-slot="call"><span class="slot-hole"></span><p class="slot-text"><b>Mr Tan's missed call.</b> Called back. Friday delivery confirmed.</p><img src="assets/icons/twilio.svg" alt="Twilio"></li>
          <li class="slot" data-slot="cal"><span class="slot-hole"></span><p class="slot-text"><b>Your morning brief.</b> Sent to your phone at 8:00.</p><img src="assets/icons/telegram.svg" alt="Telegram"></li>
          <li class="slot" data-slot="invoice-done" hidden><span class="slot-hole"></span><p class="slot-text"><b>Harbour Fish Co.</b> You approved. Confirmation sent at 8:01.</p><img src="assets/icons/gmail.svg" alt="Gmail"></li>
        </ol>
      </div>
      <div class="tray tray--wait" data-tray="wait">
        <p class="tray-h">Waiting for your yes <span data-h-wait>nothing yet</span></p>
        <ol>
          <li class="slot slot--wait" data-slot="invoice">
            <span class="slot-hole"></span>
            <p class="slot-text" data-wait-text><b>Harbour Fish Co. raised Friday's order by 8%.</b> A reply is drafted. Money decisions wait for you.</p>
            <div class="slot-act">
              <button class="b-ok" type="button" data-approve>Approve the reply</button>
              <button class="b-no" type="button" data-deny>Deny</button>
              <button class="b-undo" type="button" data-undo hidden>Undo</button>
            </div>
          </li>
        </ol>
        <p class="tray-none" data-wait-none hidden>Nothing is waiting for you.</p>
      </div>
    </div>
  </div>`;

  const tile = (icon, name, node) => `<span class="tile" data-node="${node}" data-app="${name}"><img src="assets/icons/${icon}.svg" alt="">${name}</span>`;
  const HUB_HTML = `
  <div class="hub" data-hub>
    <svg class="hub-lines" aria-hidden="true"></svg>
    <div class="hub-col hub-in">
      <p class="hub-col-l">Ask from</p>
      ${tile('telegram', 'Telegram', 'in')}${tile('x', 'X', 'in')}${tile('whatsapp', 'WhatsApp', 'in')}${tile('slack', 'Slack', 'in')}${tile('discord', 'Discord', 'in')}
    </div>
    <div class="hub-core" data-core><img src="assets/brand/nyxid-coloured-icon.svg" alt="NyxID"></div>
    <div class="hub-col hub-out">
      <p class="hub-col-l">It works in</p>
      ${tile('google-calendar', 'Google Calendar', 'out')}${tile('gmail', 'Gmail', 'out')}${tile('google-sheets', 'Google Sheets', 'out')}${tile('google-drive', 'Google Drive', 'out')}${tile('notion', 'Notion', 'out')}${tile('stripe', 'Stripe', 'out')}${tile('twilio', 'Twilio', 'out')}${tile('github', 'GitHub', 'out')}
    </div>
  </div>`;

  /* ---------- smooth scroll and nav ---------- */
  function smooth() {
    const nav = $('.nav');
    const onScroll = () => nav && nav.classList.toggle('is-stuck', scrollY > 24);
    addEventListener('scroll', onScroll, { passive: true });
    onScroll();
    if (reduced() || !window.Lenis || !has || !P.ST) return null;
    const lenis = new window.Lenis({ lerp: 0.11 });
    lenis.on('scroll', P.ST.update);
    G.ticker.add((t) => lenis.raf(t * 1000));
    G.ticker.lagSmoothing(0);
    document.addEventListener('click', (e) => {
      const a = e.target.closest('a[href^="#"]');
      const id = a && a.getAttribute('href');
      if (!id || id.length < 2) return;
      const target = $(id);
      if (!target) return;
      e.preventDefault();
      lenis.scrollTo(target, { offset: id === '#main' ? 0 : -72 });
    });
    return lenis;
  }

  /* ---------- the pile gets sorted ---------- */
  function initSort(section, hooks = {}) {
    section.innerHTML = PILE_HTML;
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
    const WAIT_HTML = waitText.innerHTML;
    function showWait() { waitSlot.hidden = false; waitNone.hidden = true; }
    function resetDecision() {
      S.decision = null;
      waitText.innerHTML = WAIT_HTML;
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
      waitText.innerHTML = '<b>Denied.</b> NyxBot sent nothing to Harbour Fish Co. The draft is still there if you change your mind.';
      bOk.hidden = bNo.hidden = true; bUndo.hidden = false;
      counts();
    });
    bUndo.addEventListener('click', () => {
      waitText.innerHTML = WAIT_HTML;
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

  /* ---------- apps hub: lines from every chat to NyxID and out to every app ---------- */
  function initHub(section, opts = {}) {
    const mount = $('[data-hub-mount]', section);
    if (!mount) return null;
    mount.outerHTML = HUB_HTML;
    const hub = $('[data-hub]', section);
    const svg = $('.hub-lines', hub);
    const api = { hub, routes: { in: [], out: [] } };
    function draw() {
      if (getComputedStyle(svg).display === 'none') { api.routes = { in: [], out: [] }; return; }
      const r = hub.getBoundingClientRect();
      const core = $('[data-core]', hub).getBoundingClientRect();
      const cy = core.top - r.top + core.height / 2;
      const cL = core.left - r.left, cR = core.right - r.left;
      svg.setAttribute('viewBox', `0 0 ${r.width} ${r.height}`);
      let html = opts.defs || '';
      const routes = { in: [], out: [] };
      const add = (d, i, kind, t, ghost) => {
        // a ghost route is invisible: travellers still use it, but it does not draw across the first column
        html += `<path data-i="${i}" class="${ghost ? 'ghost' : 'base'}" d="${d}"/>`;
        if (opts.pulse !== false && !ghost) html += `<path class="pulse" d="${d}" style="animation-delay:${(-(i * 0.83) % 3.2).toFixed(2)}s"/>`;
        routes[kind].push({ d, tile: t, i });
      };
      const tiles = $$('.tile', hub);
      const outs = tiles.filter((t) => t.dataset.node === 'out');
      const minLeft = Math.min(...outs.map((t) => t.getBoundingClientRect().left));
      tiles.forEach((t, i) => {
        const b = t.getBoundingClientRect();
        const y = b.top - r.top + b.height / 2;
        if (t.dataset.node === 'in') {
          const x = b.right - r.left, mx = (x + cL) / 2;
          add(`M${x} ${y} C${mx} ${y} ${mx} ${cy} ${cL} ${cy}`, i, 'in', t);
        } else if (b.left - minLeft < 4) {
          const x = b.left - r.left, mx = (x + cR) / 2;
          add(`M${cR} ${cy} C${mx} ${cy} ${mx} ${y} ${x} ${y}`, i, 'out', t);
        } else {
          // second column: a short visible stub, plus a ghost route travellers can follow
          const x = b.left - r.left, mx = (x + cR) / 2;
          html += `<path class="base stub" d="M${x - 10} ${y} L${x} ${y}"/>`;
          add(`M${cR} ${cy} C${mx} ${cy} ${mx} ${y} ${x} ${y}`, i, 'out', t, true);
        }
      });
      svg.innerHTML = html;
      api.routes = routes;
      tiles.forEach((t, i) => {
        const path = $(`path[data-i="${i}"]`, svg);
        t.onmouseenter = () => path && path.classList.add('is-hot');
        t.onmouseleave = () => path && path.classList.remove('is-hot');
      });
      if (opts.onDraw) opts.onDraw(api);
    }
    api.draw = draw;
    draw();
    let t;
    addEventListener('resize', () => { clearTimeout(t); t = setTimeout(draw, 150); });
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(draw).catch(() => {});
    return api;
  }

  /* send an element along a hub route: chat tile -> core -> app tile */
  function travel(api, el, { duration = 1.6, rotate = false, onArrive } = {}) {
    const { routes, hub } = api;
    if (!has || !P.Motion || !routes.in.length || !routes.out.length) return null;
    const a = routes.in[Math.floor(Math.random() * routes.in.length)];
    const b = routes.out[Math.floor(Math.random() * routes.out.length)];
    hub.appendChild(el);
    a.tile.classList.add('is-hit');
    const tl = G.timeline({ onComplete: () => { el.remove(); b.tile.classList.remove('is-hit'); if (onArrive) onArrive(b.tile); } });
    tl.set(el, { opacity: 1 })
      .to(el, { duration: duration * 0.45, ease: 'power1.in', motionPath: { path: a.d, autoRotate: rotate, alignOrigin: [0.5, 0.5] } })
      .add(() => a.tile.classList.remove('is-hit'))
      .to(el, { duration: duration * 0.55, ease: 'power2.out', motionPath: { path: b.d, autoRotate: rotate, alignOrigin: [0.5, 0.5] } })
      .add(() => b.tile.classList.add('is-hit'), '-=0.05');
    return tl;
  }

  /* run a callback only while an element is on screen */
  function whileVisible(el, start, stop) {
    if (!('IntersectionObserver' in window)) { start(); return; }
    let on = false;
    new IntersectionObserver(([e]) => {
      if (e.isIntersecting && !on) { on = true; start(); }
      else if (!e.isIntersecting && on) { on = false; stop(); }
    }, { rootMargin: '80px' }).observe(el);
  }

  window.NyxKit = { G, P, has, $, $$, clamp, reduced, smooth, initSort, initHub, travel, whileVisible };
})();
