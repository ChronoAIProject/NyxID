// Keep the program literal: serializing a compiled function can retain Metro or
// TSX helper references that do not exist inside the isolated WebView.
const program = String.raw`
(() => {
  const canvas = document.getElementById('desktop');
  const context = canvas.getContext('2d', { alpha: false });
  let generation = 0, last = 0n, pending, pendingUrl, scale = 1, controls = false, offsetX = 0, offsetY = 0;
  const bridge = value => window.ReactNativeWebView.postMessage(JSON.stringify(value));
  if (!context) { bridge({type: 'unavailable'}); return; }
  const layout = () => {
    const fit = Math.min(window.innerWidth / canvas.width, window.innerHeight / canvas.height);
    if (!Number.isFinite(fit)) return;
    canvas.style.width = (canvas.width * fit * scale) + 'px';
    canvas.style.height = (canvas.height * fit * scale) + 'px';
    canvas.style.transform = 'translate(' + offsetX + 'px,' + offsetY + 'px)';
  };
  window.addEventListener('resize', layout);
  const point = touch => {
    const rect = canvas.getBoundingClientRect();
    if (!canvas.width || !rect.width || touch.clientX < rect.left || touch.clientY < rect.top || touch.clientX >= rect.right || touch.clientY >= rect.bottom) return;
    return {x: (touch.clientX - rect.left) * canvas.width / rect.width, y: (touch.clientY - rect.top) * canvas.height / rect.height};
  };
  let gesture;
  const distance = touches => Math.hypot(touches[0].clientX - touches[1].clientX, touches[0].clientY - touches[1].clientY);
  const center = touches => ({clientX: (touches[0].clientX + touches[1].clientX) / 2, clientY: (touches[0].clientY + touches[1].clientY) / 2});
  const input = (tool, args) => { if (controls) bridge({type:'input', input:{tool, arguments:args}}); };
  document.addEventListener('touchstart', event => {
    event.preventDefault(); bridge({type:'interaction'});
    if (event.touches.length === 1) {
      const touch = event.touches[0];
      gesture = {mode:'single', start:point(touch), x:touch.clientX, y:touch.clientY, at:Date.now(), offsetX, offsetY};
    } else if (event.touches.length === 2) {
      gesture = {mode:'pair', distance:distance(event.touches), center:center(event.touches), scale, kind:undefined, at:0};
    } else gesture = undefined;
  }, {passive:false});
  document.addEventListener('touchmove', event => {
    event.preventDefault();
    if (!gesture) return;
    if (gesture.mode === 'single' && event.touches.length === 1 && !controls && scale > 1) {
      offsetX = gesture.offsetX + event.touches[0].clientX - gesture.x;
      offsetY = gesture.offsetY + event.touches[0].clientY - gesture.y; layout();
    }
    if (gesture.mode === 'pair' && event.touches.length === 2) {
      const next = center(event.touches), nextDistance = distance(event.touches);
      const dx = next.clientX - gesture.center.clientX, dy = next.clientY - gesture.center.clientY;
      if (!gesture.kind) {
        if (Math.abs(nextDistance - gesture.distance) > 12) gesture.kind = 'pinch';
        else if (Math.hypot(dx,dy) > 12) gesture.kind = 'scroll';
      }
      if (gesture.kind === 'pinch') {
        scale = Math.min(5,Math.max(1,gesture.scale * nextDistance / Math.max(1,gesture.distance)));
        if (scale === 1) {offsetX = 0;offsetY = 0;} layout();
      } else if (gesture.kind === 'scroll' && Date.now() - gesture.at > 80) {
        const position = point(next);
        if (position && Math.hypot(dx,dy) > 8) input('scroll', {...position,direction:Math.abs(dx)>Math.abs(dy)?(dx>0?'left':'right'):(dy>0?'up':'down'),amount:3});
        gesture.center = next; gesture.at = Date.now();
      }
    }
  }, {passive:false});
  document.addEventListener('touchend', event => {
    event.preventDefault();
    const start = gesture; gesture = undefined;
    if (!start || start.mode !== 'single' || event.touches.length || !start.start) return;
    const end = point(event.changedTouches[0]);
    if (!end) return;
    const moved = Math.hypot(end.x - start.start.x,end.y - start.start.y), elapsed = Date.now() - start.at;
    if (controls && moved > 4) input('drag', {from_x:start.start.x,from_y:start.start.y,to_x:end.x,to_y:end.y,button:'left',duration_ms:Math.min(10000,elapsed)});
    else if (controls) input('click', {...end,button:elapsed>=500?'right':'left',count:1});
    else if (moved <= 4) bridge({type:'toggleOverlay'});
  }, {passive:false});
  document.addEventListener('touchcancel', () => {gesture=undefined;});
  document.addEventListener('contextmenu', event => event.preventDefault());
  window.nyxDesktop = {
    clear(next) {
      generation = next; last = 0n; controls = false; gesture = undefined; scale = 1; offsetX = 0; offsetY = 0;
      if (pending) { pending.onload = null; pending.onerror = null; pending.src = ''; pending = undefined; }
      if (pendingUrl) { URL.revokeObjectURL(pendingUrl); pendingUrl = undefined; }
      canvas.width = 0; canvas.height = 0;
    },
    control(value) { const next = value === true; if (next !== controls) gesture = undefined; controls = next; },
    zoom(value) { if (value >= 1 && value <= 5) { scale = value; layout(); } },
    frame(frame) {
      if (frame.generation !== generation || pending) return;
      const image = new Image(); pending = image;
      const finish = ok => {
        if (frame.generation !== generation || pending !== image) return;
        image.onload = null; image.onerror = null; image.src = ''; pending = undefined;
        URL.revokeObjectURL(pendingUrl); pendingUrl = undefined;
        bridge({ type: 'decoded', generation, sequence: frame.sequence, ok });
      };
      image.onerror = () => finish(false);
      image.onload = () => {
        if (frame.generation !== generation || pending !== image) return;
        const r = frame.rectangle;
        if (image.naturalWidth !== frame.width || image.naturalHeight !== frame.height ||
          frame.width < 1 || frame.height < 1 || frame.width > 1920 || frame.height > 1200 ||
          (r && (r.width < 1 || r.height < 1 || r.width > 1920 || r.height > 1200 ||
            r.x < 0 || r.y < 0 || r.x + frame.width > r.width || r.y + frame.height > r.height)) ||
          (r && r.base !== '0' && (BigInt(r.base) !== last || r.width !== canvas.width || r.height !== canvas.height))) {
          finish(false); return;
        }
        if (!r || r.base === '0') {
          if (r && (r.x || r.y || r.width !== frame.width || r.height !== frame.height)) { finish(false); return; }
          canvas.width = r ? r.width : frame.width; canvas.height = r ? r.height : frame.height;
        }
        context.drawImage(image, r ? r.x : 0, r ? r.y : 0);
        last = BigInt(frame.sequence); layout(); finish(true);
      };
      try {
        const raw = atob(frame.jpeg);
        const bytes = Uint8Array.from(raw, value => value.charCodeAt(0));
        pendingUrl = URL.createObjectURL(new Blob([bytes], { type: 'image/jpeg' }));
        image.src = pendingUrl;
      } catch { finish(false); }
    }
  };
  bridge({ type: 'ready' });
})();`;

export function canvasDocument(): string {
  return `<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src blob:; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'"><style>html,body{margin:0;background:#000;width:100%;height:100%;overflow:hidden;touch-action:none;user-select:none;-webkit-user-select:none}main{width:100%;height:100%;display:flex;align-items:center;justify-content:center}canvas{display:block;object-fit:contain}</style></head><body><main><canvas id="desktop" width="0" height="0" aria-label="Machine desktop. Take control to use touch and keyboard."></canvas></main><script>${program}</script></body></html>`;
}
