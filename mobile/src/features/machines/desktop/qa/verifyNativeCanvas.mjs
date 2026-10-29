// Run from the repository root, after A releases the simulator:
// mkdir ~/.heca/plans/pr1866-native-machines/sim.lock && \
//   /opt/homebrew/bin/node mobile/src/features/machines/desktop/qa/verifyNativeCanvas.mjs
// The runner uses only synthetic pixels and restores the registered main app.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { writeFile, unlink, rmdir } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { RED_JPEG, GREEN_PATCH, BLUE_JPEG, vector } from '../testVectors.ts';

const require = createRequire(new URL('../../../../../package.json', import.meta.url));
const WebSocket = require('ws');
const lock = '/Users/chronoai/.heca/plans/pr1866-native-machines/sim.lock';
let ownsLock = false;
let socket;
let restoreDebugger;

async function nativeProof(vectors) {
  const modules = Array.from(globalThis.__r.getModules());
  const load = name => {
    const entry = modules.find(([, value]) => value.verboseName === name);
    if (!entry) throw new Error('Missing loaded module: ' + name);
    return globalThis.__r(entry[0]);
  };
  const React = load('node_modules/react/index.js');
  const RN = load('node_modules/react-native/index.js');
  const { AppRegistry } = RN;
  const web = load('node_modules/react-native-webview/src/WebView.ios.tsx');
  const WebView = web.default || web.WebView;
  const { canvasDocument } = load('src/features/machines/desktop/canvasDocument.ts');
  const { createCompositor } = load('src/features/machines/desktop/compositor.ts');
  const protocol = load('src/features/machines/desktop/protocol.ts');
  const roots = Array.from(globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__.renderers)
    .flatMap(([id]) => Array.from(globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__.getFiberRoots(id)));
  const rootTag = roots.map(root => root.containerInfo?.containerTag).find(tag => typeof tag === 'number');
  if (!rootTag || !AppRegistry.getRunnable('main')) throw new Error('No existing main application root');
  const check = (condition, message) => { if (!condition) throw new Error(message); };
  const waiters = new Map();
  const wait = key => new Promise((resolve, reject) => {
    const timeout = setTimeout(() => { waiters.delete(key); reject(new Error('Native proof timeout: ' + key)); }, 15000);
    waiters.set(key, value => { clearTimeout(timeout); resolve(value); });
  });
  const emit = (key, value) => { const resolve = waiters.get(key); waiters.delete(key); resolve?.(value); };
  let view;
  let generation;
  let restored = false;
  let refreshes = 0;
  const acks = [];
  const inputs = [];
  const requests = [];
  const inject = source => { check(!restored, 'Native proof disposed'); view.injectJavaScript(source + ';true;'); };
  const compositor = createCompositor(frame => inject('window.nyxDesktop.frame(' + JSON.stringify(frame) + ')'), () => refreshes++);
  const restore = () => {
    if (restored) return;
    restored = true; compositor.reset();
    view?.injectJavaScript('window.nyxDesktop?.clear(2147483647);true;');
    AppRegistry.runApplication('main', { rootTag, initialProps: {}, fabric: true });
  };
  globalThis.__nyxNativeCanvasRestore = restore;
  const instrumentation = `<script>
    window.__proofUrls = 0; window.__proofPeak = 0;
    const createProofUrl = URL.createObjectURL.bind(URL), revokeProofUrl = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = value => { window.__proofUrls++; window.__proofPeak = Math.max(window.__proofPeak, window.__proofUrls); return createProofUrl(value); };
    URL.revokeObjectURL = value => { window.__proofUrls--; return revokeProofUrl(value); };
  </script>`;
  const html = canvasDocument().replace('<script>', instrumentation + '<script>');
  function Proof() {
    return React.createElement(RN.View, { style: { flex: 1, backgroundColor: '#000' } },
      React.createElement(RN.Text, { style: { color: '#fff', padding: 30 } }, 'Synthetic desktop renderer proof'),
      React.createElement(WebView, {
        ref: value => { view = value; }, source: { html, baseUrl: 'about:blank' }, style: { flex: 1 },
        originWhitelist: ['about:blank'], javaScriptEnabled: true, domStorageEnabled: false,
        cacheEnabled: false, incognito: true, sharedCookiesEnabled: false, allowFileAccess: false,
        onShouldStartLoadWithRequest: request => { if (request.url !== 'about:blank') requests.push(request.url); return request.url === 'about:blank'; },
        onMessage: event => {
          const value = JSON.parse(event.nativeEvent.data);
          if (value.type === 'decoded') {
            compositor.acknowledge(value); acks.push(value); emit('decoded:' + value.sequence, value);
          } else if (value.type === 'input') inputs.push(value.input);
          else emit(value.type, value);
        },
      }));
  }
  const ready = wait('ready');
  AppRegistry.registerComponent('NyxDesktopNativeProof', () => Proof);
  try {
    AppRegistry.runApplication('NyxDesktopNativeProof', { rootTag, initialProps: {}, fabric: true });
    await ready;
    generation = compositor.reset(); inject('window.nyxDesktop.clear(' + generation + ')');
    const parse = bytes => {
      const frame = protocol.desktopFrame(Uint8Array.from(bytes).buffer, '00112233-4455-6677-8899-aabbccddeeff');
      check(frame, 'Native parser refused a shared wire vector'); return frame;
    };
    check(parse(vectors.big).sequence.toString() === '9007199254740993', 'Native parser preserves the complete u64 sequence');
    const send = async frame => {
      const done = wait('decoded:' + frame.sequence.toString()); compositor.enqueue(frame);
      const result = await done; check(result.ok, 'Native image decode failed'); return result;
    };
    const probe = async () => {
      const done = wait('probe');
      inject(`(() => { const c=document.querySelector('canvas'),x=c.getContext('2d'); const p=(a,b)=>Array.from(x.getImageData(a,b,1,1).data); window.ReactNativeWebView.postMessage(JSON.stringify({type:'probe',width:c.width,height:c.height,first:c.width?p(0,0):[],patch:c.width>2&&c.height>2?p(2,2):[],urls:window.__proofUrls,peak:window.__proofPeak})); })()`);
      return done;
    };
    await send(parse(vectors.red)); const red = await probe();
    check(red.first[0] > 240 && red.first[1] < 15 && red.first[2] < 15, 'Full red JPEG pixels');
    await send(parse(vectors.patch)); const patch = await probe();
    check(patch.patch[1] > 240 && patch.patch[0] < 15 && JSON.stringify(patch.first) === JSON.stringify(red.first), 'Patch reconstruction and unaffected pixels');
    await send(parse(vectors.blue)); const blue = await probe();
    check(blue.width === 4 && blue.height === 3 && blue.first[2] > 240, 'Full frame resets surface');
    const invalid = wait('decoded:4');
    inject('window.nyxDesktop.frame(' + JSON.stringify({ generation, sequence: '4', jpeg: vectors.green, width: 1920, height: 1200 }) + ')');
    check(!(await invalid).ok, 'Actual decoded dimensions must be checked');
    const corrupt = wait('decoded:4');
    inject('window.nyxDesktop.frame(' + JSON.stringify({ generation, sequence: '4', jpeg: vectors.corrupt, width: 8, height: 6 }) + ')');
    check(!(await corrupt).ok, 'Corrupt JPEG must be refused');
    check(JSON.stringify((await probe()).first) === JSON.stringify(blue.first), 'Invalid decodes cannot repaint');

    const fixture = wait('fixture');
    inject(`(() => { const c=document.createElement('canvas');c.width=1920;c.height=1200;const x=c.getContext('2d'),p=x.createImageData(1920,1200);let seed=17;for(let i=0;i<p.data.length;i+=4){seed=(seed*1664525+1013904223)>>>0;p.data[i]=seed&255;p.data[i+1]=(seed>>>8)&255;p.data[i+2]=(seed>>>16)&255;p.data[i+3]=255;}x.putImageData(p,0,0);window.ReactNativeWebView.postMessage(JSON.stringify({type:'fixture',jpeg:c.toDataURL('image/jpeg',0.9).split(',')[1]}));c.width=0;c.height=0; })()`);
    let large = (await fixture).jpeg;
    const bytes = Uint8Array.from(atob(large), char => char.charCodeAt(0));
    const jpegBytes = bytes.length;
    const bridgeCharacters = large.length;
    large = undefined;
    const timings = [];
    for (let index = 0; index < 20; index++) {
      globalThis.__nyxNativeCanvasFrame = index;
      const start = performance.now();
      await send({ sequence: BigInt(5 + index), end: false, jpeg: bytes, width: 1920, height: 1200 });
      timings.push(performance.now() - start);
    }
    const gestures = wait('gestures');
    inject(`(() => {
      const c=document.querySelector('canvas');
      const touch=(x,y)=>{const r=c.getBoundingClientRect();return {clientX:r.left+x*r.width/c.width,clientY:r.top+y*r.height/c.height};};
      const fire=(type,touches,changedTouches=touches)=>{const e=new Event(type,{cancelable:true});Object.defineProperties(e,{touches:{value:touches},changedTouches:{value:changedTouches}});document.dispatchEvent(e);};
      const a=touch(100,100),b=touch(300,250);
      window.nyxDesktop.control(false);fire('touchstart',[a]);fire('touchend',[],[a]);
      window.nyxDesktop.control(true);fire('touchstart',[a]);fire('touchend',[],[a]);
      fire('touchstart',[a]);
      setTimeout(()=>{
        fire('touchend',[],[a]);
        fire('touchstart',[a]);fire('touchend',[],[b]);
        fire('touchstart',[touch(600,300),touch(900,300)]);
        fire('touchmove',[touch(600,600),touch(900,600)]);fire('touchend',[],[touch(600,600)]);
        fire('touchstart',[touch(600,300),touch(900,300)]);
        fire('touchmove',[touch(300,300),touch(1200,300)]);fire('touchend',[],[touch(300,300)]);
        window.nyxDesktop.zoom(1);window.nyxDesktop.control(false);
        window.ReactNativeWebView.postMessage(JSON.stringify({type:'gestures'}));
      },600);
    })()`);
    await gestures;
    check(JSON.stringify(inputs.map(value=>value.tool))===JSON.stringify(['click','click','drag','scroll']), 'Native canvas control gate, pointer gestures and local-only pinch');
    check(inputs[0].arguments.button==='left'&&inputs[1].arguments.button==='right', 'Native tap and long-press click buttons');

    // Cancel a real pending native decode before its callback can repaint.
    const previousAcks = acks.length;
    inject('window.nyxDesktop.frame(' + JSON.stringify({ generation, sequence: '25', jpeg: protocol.base64(bytes), width: 1920, height: 1200 }) + ');window.nyxDesktop.clear(' + (generation + 1) + ')');
    await new Promise(resolve => setTimeout(resolve, 250));
    const cleared = await probe();
    check(acks.length === previousAcks && cleared.width === 0 && cleared.height === 0 && cleared.urls === 0 && cleared.peak === 1, 'Cancellation releases URLs and pixels without stale callback');

    generation = compositor.reset(); inject('window.nyxDesktop.clear(' + generation + ')');
    compositor.enqueue(parse(vectors.red));
    for (let sequence = 2; sequence < 8; sequence++) {
      const frame = parse(vectors.patch); frame.sequence = BigInt(sequence); frame.rectangle.base = BigInt(sequence - 1);
      compositor.enqueue(frame);
    }
    const queueDepth = compositor.depth();
    check(queueDepth === 4 && refreshes >= 1, 'Native decode queue is bounded to four');
    await new Promise(resolve => setTimeout(resolve, 1000));
    generation = compositor.reset(); inject('window.nyxDesktop.clear(' + generation + ')');
    const disposed = await probe(); check(disposed.width === 0 && disposed.urls === 0, 'Final native disposal');
    check(requests.length === 0, 'Renderer must not navigate to a network origin');
    return { red, patch, fullReset: blue, u64RoundTrip: true, decodedSizeMismatchRefused: true, corruptJpegRefused: true,
      touchInputTools: inputs.map(value=>value.tool), nativeControlGate: true, nativePinchLocalOnly: true,
      jpegBytes, bridgeCharacters, frames: timings.length,
      meanBridgeDecodeMs: timings.reduce((a, b) => a + b) / timings.length,
      maxBridgeDecodeMs: Math.max(...timings), queueDepth, refreshes, cleared, disposed, requests };
  } finally {
    restore(); delete globalThis.__nyxNativeCanvasRestore; delete globalThis.__nyxNativeCanvasFrame;
  }
}

try {
  await writeFile(lock + '/owner', 'B native canvas proof ' + new Date().toISOString(), { flag: 'wx' }); ownsLock = true;
  const memory = () => {
    const processes = execFileSync('ps', ['-axo', 'pid,rss,comm'], { encoding: 'utf8' }).split('\n')
      .map(line => /^\s*(\d+)\s+(\d+)\s+(.+)$/.exec(line)).filter(Boolean)
      .map(match => ({ pid: Number(match[1]), residentKiB: Number(match[2]), command: match[3] }));
    return { app: processes.filter(value => value.command.includes('/2171D3CF-8D01-4693-8F85-7219CADEE652/') && value.command.endsWith('/NyxIDMobile.app/NyxIDMobile')),
      web: processes.filter(value => value.command.includes('WebContent')) };
  };
  const baseline = memory();
  const existingWebPids = new Set(baseline.web.map(value => value.pid));
  const sample = frame => {
    const current = memory();
    return { frame, appKiB: current.app.reduce((sum, value) => sum + value.residentKiB, 0),
      newWebKiB: current.web.filter(value => !existingWebPids.has(value.pid)).reduce((sum, value) => sum + value.residentKiB, 0) };
  };
  const baselineSample = sample(undefined);
  const samples = [];
  const pages = await (await fetch('http://localhost:8091/json/list')).json();
  const page = pages.find(value => value.description.startsWith('React Native Bridgeless'));
  assert.ok(page, 'Running iOS development application');
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.once('open', resolve); socket.once('error', reject); });
  let requestId = 0;
  const evaluate = expression => new Promise((resolve, reject) => {
    const id = ++requestId;
    const timer = setTimeout(() => { socket.off('message', message); reject(new Error('Native debugger request deadline')); }, 30000);
    const message = raw => {
      const value = JSON.parse(raw);
      if (value.id !== id) return;
      clearTimeout(timer); socket.off('message', message);
      if (value.result?.exceptionDetails) reject(new Error(value.result.exceptionDetails.text));
      else resolve(value.result?.result?.value);
    };
    socket.on('message', message);
    socket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true } }));
  });
  restoreDebugger = () => evaluate("if (typeof globalThis.__nyxNativeCanvasRestore === 'function') globalThis.__nyxNativeCanvasRestore(); void 0");
  const vectors = {
    red: Array.from(new Uint8Array(vector(new Uint8Array(Buffer.from(RED_JPEG, 'base64')), 1n))),
    big: Array.from(new Uint8Array(vector(new Uint8Array(Buffer.from(RED_JPEG, 'base64')), 9007199254740993n))),
    patch: Array.from(new Uint8Array(vector(new Uint8Array(Buffer.from(GREEN_PATCH, 'base64')), 2n,
      { width: 8, height: 6, x: 2, y: 2, patchWidth: 2, patchHeight: 2, base: 1n }))),
    blue: Array.from(new Uint8Array(vector(new Uint8Array(Buffer.from(BLUE_JPEG, 'base64')), 3n))),
    green: GREEN_PATCH,
    corrupt: Buffer.from([255,216,255,192,0,17,8,0,6,0,8,3,1,17,0,2,17,0,3,17,0,255,217]).toString('base64'),
  };
  // Metro normally transforms async syntax for Hermes. The debugger's direct
  // evaluator needs that same transform, including its inline helper.
  const expression = require('@babel/core').transformSync(
    'globalThis.__nyxNativeCanvasProof = undefined; (' + nativeProof.toString() + ')(' + JSON.stringify(vectors) + ')' +
      '.then(value => { globalThis.__nyxNativeCanvasProof = { ok: true, value }; }, error => { globalThis.__nyxNativeCanvasProof = { ok: false, error: String(error) }; }); void 0;',
    { configFile: false, babelrc: false, sourceType: 'script', presets: [[require('@react-native/babel-preset'),
      { dev: true, unstable_transformProfile: 'hermes-stable', enableBabelRuntime: false }]] },
  ).code;
  await evaluate(expression);
  const deadline = Date.now() + 600000;
  let result;
  while (Date.now() < deadline) {
    const state = await evaluate('({result:globalThis.__nyxNativeCanvasProof,frame:globalThis.__nyxNativeCanvasFrame})');
    samples.push(sample(state.frame));
    result = state.result;
    if (result) break;
    await new Promise(resolve => setTimeout(resolve, 500));
  }
  assert.ok(result, 'Native canvas proof deadline');
  await evaluate('delete globalThis.__nyxNativeCanvasProof');
  assert.ok(result.ok, result.error);
  assert.ok(result.value?.fullReset, 'Complete native canvas proof result');
  await new Promise(resolve => setTimeout(resolve, 2000));
  console.log(JSON.stringify({ ...result.value, memory: { baseline: baselineSample, afterRestore: sample(undefined),
    peakAppKiB: Math.max(...samples.map(value => value.appKiB)), peakNewWebKiB: Math.max(...samples.map(value => value.newWebKiB)), samples } }, null, 2));
} finally {
  if (socket?.readyState === WebSocket.OPEN) await restoreDebugger?.().catch(() => {});
  socket?.close();
  if (ownsLock) { await unlink(lock + '/owner'); await rmdir(lock); }
}
