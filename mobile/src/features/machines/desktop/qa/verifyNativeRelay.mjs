// Start the local production backend on 3011 with its own temporary database.
// A/PM must serve Metro with EXPO_PUBLIC_API_BASE_URL=http://127.0.0.1:3011/api/v1
// for this run. The runner refuses to redirect or bypass the app transport.
// From the repository root, after the simulator is released:
// mkdir ~/.heca/plans/pr1866-native-machines/sim.lock && \
//   /opt/homebrew/bin/node mobile/src/features/machines/desktop/qa/verifyNativeRelay.mjs
// No frame files, screenshots, app credentials or token-bearing output.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { writeFile, unlink, rmdir } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { randomUUID } from 'node:crypto';
import { relayFixture } from './relayFixture.mjs';

const require = createRequire(new URL('../../../../../package.json', import.meta.url));
const WebSocket = require('ws');
const execute = promisify(execFile);
const lock = '/Users/chronoai/.heca/plans/pr1866-native-machines/sim.lock';
const device = '2171D3CF-8D01-4693-8F85-7219CADEE652';
let ownsLock = false;
let fixture;
let debuggerSocket;
let evaluate;
let appBackgrounded = false;

async function nativeRelay(session, params, missingNode) {
  const entries = Array.from(globalThis.__r.getModules());
  const load = name => {
    const names = Array.isArray(name) ? name : [name];
    const entry = entries.find(([, value]) => names.includes(value.verboseName));
    if (!entry) throw new Error('Loaded module missing: ' + names.join(', '));
    return globalThis.__r(entry[0]);
  };
  const React = load('node_modules/react/index.js');
  const RN = load('node_modules/react-native/index.js');
  const { NavigationContainer, createNavigationContainerRef } = load([
    'node_modules/@react-navigation/native/src/index.tsx', 'node_modules/@react-navigation/native/lib/module/index.js']);
  const { createNativeStackNavigator } = load([
    'node_modules/@react-navigation/native-stack/src/index.tsx', 'node_modules/@react-navigation/native-stack/lib/module/index.js']);
  const { SafeAreaProvider } = load('node_modules/react-native-safe-area-context/src/index.tsx');
  const { ThemeProvider } = load('src/theme/ThemeContext.tsx');
  const { MachineDesktopRoute } = load('src/features/machines/MachineDesktopRoute.tsx');
  const { desktopPrivacyQueue } = load('src/features/machines/desktopPrivacy.ts');
  const store = load('src/lib/auth/sessionStore.ts');
  const epochs = load('src/lib/auth/authEpoch.ts');
  const env = load('src/lib/env.ts');
  if (!/^http:\/\/127\.0\.0\.1:3011\/api\/v1(?:\/mobile)?$/.test(env.NYXID_API_BASE_URL)) {
    throw new Error('A/PM must serve Metro with the local backend API base on 3011 before real relay QA');
  }
  const compositorModule = load('src/features/machines/desktop/compositor.ts');
  const viewerModule = load('src/features/machines/desktop/viewer.ts');
  const roots = () => Array.from(globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__.renderers)
    .flatMap(([id]) => Array.from(globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__.getFiberRoots(id)));
  const rootTag = roots().map(root => root.containerInfo?.containerTag).find(tag => typeof tag === 'number');
  if (!rootTag || !RN.AppRegistry.getRunnable('main')) throw new Error('No running application root');
  const original = { compositor: compositorModule.createCompositor, viewer: viewerModule.createDesktopViewer };
  const savedSession = await store.loadStoredAuthSession();
  const navigation = createNavigationContainerRef();
  const Stack = createNativeStackNavigator();
  const proof = { stage: 'mounting', decoded: [], phases: [], messages: [], closes: 0, resets: 0, socketAttempts: 0,
    backgroundSeen: false, backgroundClosed: false, foregroundReopened: false, blurClosed: false, epochClosed: false,
    signOutClosed: false, closeActionClosed: false, refusalVisible: false, controls: false,
    landscape: false, portraitRestored: false, explicitHandBack: false, backgroundHandBack: false };
  globalThis.__nyxRelayProof = proof;
  const appStateObserver = RN.AppState.addEventListener('change', next => { if (next !== 'active') proof.backgroundSeen = true; });
  let activeViewer;
  let restored = false;
  const wait = async (condition, description) => {
    const deadline = Date.now() + 25000;
    while (!condition()) {
      if (Date.now() >= deadline || restored) throw new Error('Native relay deadline: ' + description);
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  };
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const element = label => {
    const walk = fiber => {
      if (!fiber) return undefined;
      if (fiber.memoizedProps?.accessibilityLabel === label) return fiber.memoizedProps;
      return walk(fiber.child) || walk(fiber.sibling);
    };
    for (const root of roots()) { const action = walk(root.current); if (action) return action; }
    return undefined;
  };
  const button = label => element(label)?.onPress;
  const rendererInput = input => {
    const walk = fiber => {
      if (!fiber) return undefined;
      if (fiber.memoizedProps?.incognito && typeof fiber.memoizedProps?.onMessage === 'function') return fiber.memoizedProps.onMessage;
      return walk(fiber.child) || walk(fiber.sibling);
    };
    for (const root of roots()) {
      const receive = walk(root.current);
      if (receive) { receive({ nativeEvent: { data: JSON.stringify({ type: 'input', input }) } }); return; }
    }
    throw new Error('Production WebView bridge is absent');
  };
  const Blank = () => React.createElement(RN.View, { style: { flex: 1, backgroundColor: '#000' } });
  const Proof = () => React.createElement(SafeAreaProvider, null,
    React.createElement(ThemeProvider, null,
      React.createElement(NavigationContainer, { ref: navigation },
        React.createElement(Stack.Navigator, { initialRouteName: 'Blank', screenOptions: { headerShown: false, animation: 'none' } },
          React.createElement(Stack.Screen, { name: 'Blank', component: Blank }),
          React.createElement(Stack.Screen, { name: 'MachineDesktop', component: MachineDesktopRoute })))));
  const restore = async () => {
    if (restored) return;
    restored = true;
    appStateObserver.remove();
    activeViewer?.stop();
    RN.AppRegistry.registerComponent('NyxDesktopRelayBlank', () => Blank);
    RN.AppRegistry.runApplication('NyxDesktopRelayBlank', { rootTag, initialProps: {}, fabric: true });
    await pause(300);
    await desktopPrivacyQueue(async () => {});
    compositorModule.createCompositor = original.compositor; viewerModule.createDesktopViewer = original.viewer;
    if (savedSession) await store.persistAuthSession(savedSession); else await store.clearStoredAuthSession();
    RN.AppRegistry.runApplication('main', { rootTag, initialProps: {}, fabric: true });
  };
  globalThis.__nyxRelayRestore = restore;
  try {
    // Observe the production route/compositor/viewer without replacing auth,
    // HTTP requests, socket construction, URL building or native protection.
    compositorModule.createCompositor = (send, refresh) => {
      const compositor = original.compositor(send, refresh);
      const acknowledge = compositor.acknowledge;
      compositor.acknowledge = value => {
        const accepted = acknowledge(value);
        proof.decoded.push({ sequence: value.sequence, ok: value.ok, accepted });
        return accepted;
      };
      const reset = compositor.reset;
      compositor.reset = () => { proof.resets++; return reset(); };
      return compositor;
    };
    viewerModule.createDesktopViewer = deps => {
      const viewer = original.viewer({ ...deps,
        state: value => { proof.phases.push(value.phase); proof.controls = value.controls === true; deps.state(value); },
        connect: async () => {
          const ws = await deps.connect(); proof.socketAttempts++;
          const send = ws.send.bind(ws), close = ws.close.bind(ws);
          ws.send = value => {
            if (typeof value === 'string') proof.messages.push({ type: JSON.parse(value).type });
            else {
              const bytes = new Uint8Array(value);
              const input = JSON.parse(Array.from(bytes.slice(30), byte => String.fromCharCode(byte)).join(''));
              proof.messages.push({ type: 'input', tool: input.tool, acknowledged: proof.controls });
            }
            send(value);
          };
          ws.close = () => { proof.closes++; close(); };
          return ws;
        },
      });
      activeViewer = viewer;
      return viewer;
    };
    await store.persistAuthSession(session);
    RN.AppRegistry.registerComponent('NyxDesktopRelayProof', () => Proof);
    RN.AppRegistry.runApplication('NyxDesktopRelayProof', { rootTag, initialProps: {}, fabric: true });
    await wait(() => navigation.isReady(), 'navigation ready');
    const open = async (next = params) => {
      const before = proof.decoded.length;
      navigation.navigate('MachineDesktop', next);
      if (next.nodeId === params.nodeId) await wait(() => proof.decoded.length >= before + 3, 'full/patch/reset decoded');
    };
    await open();
    if (proof.decoded.some(value => !value.ok || !value.accepted)) throw new Error('Real relay frame decode failed');
    await wait(() => RN.Dimensions.get('window').width > RN.Dimensions.get('window').height, 'desktop is landscape');
    proof.landscape = true;
    const click = { tool: 'click', arguments: { x: 1, y: 1, button: 'left', count: 1 } };
    rendererInput(click); await pause(100);
    if (proof.messages.some(value => value.type === 'input')) throw new Error('Input escaped before explicit human takeover');
    button('Take control')();
    await wait(() => proof.controls && !!button('Hand back'), 'explicit takeover acknowledged');
    for (const input of [click,
      { tool: 'drag', arguments: { from_x: 1, from_y: 1, to_x: 3, to_y: 3, button: 'left', duration_ms: 300 } },
      { tool: 'scroll', arguments: { x: 1, y: 1, direction: 'down', amount: 3 } },
    ]) rendererInput(input);
    button('Keyboard')(); await wait(() => !!element('Desktop text to send'), 'keyboard is shown');
    element('Desktop text to send').onChangeText('synthetic');
    await wait(() => element('Send')?.accessibilityState?.disabled === false, 'text is ready');
    button('Send')(); button('ENTER')();
    await wait(() => proof.messages.filter(value => value.type === 'input').length >= 5, 'pointer/text/key inputs sent');
    button('Hand back')(); await wait(() => !proof.controls, 'explicit hand-back'); proof.explicitHandBack = true;
    button('Take control')(); await wait(() => proof.controls, 'background takeover acknowledged');
    const handBacks = proof.messages.filter(value => value.type === 'hand_back').length;
    const initialClose = proof.closes, initialReset = proof.resets, initialFrames = proof.decoded.length;
    proof.stage = 'initial-frames';
    await wait(() => proof.backgroundSeen && proof.closes > initialClose && proof.resets > initialReset, 'background closes and clears');
    proof.backgroundClosed = true; proof.backgroundHandBack = proof.messages.filter(value => value.type === 'hand_back').length > handBacks; proof.stage = 'background-closed';
    await wait(() => RN.AppState.currentState === 'active' && proof.decoded.length >= initialFrames + 3, 'foreground gets fresh frames');
    proof.foregroundReopened = true;
    let closes = proof.closes;
    navigation.navigate('Blank');
    await wait(() => proof.closes > closes, 'blur closes'); proof.blurClosed = true;
    await open();
    closes = proof.closes; epochs.advanceAuthEpoch();
    await wait(() => proof.closes > closes, 'auth epoch closes'); proof.epochClosed = true;
    navigation.navigate('Blank'); await pause(300); await open();
    closes = proof.closes; await store.clearStoredAuthSession();
    await wait(() => proof.closes > closes, 'sign out closes'); proof.signOutClosed = true;
    await store.persistAuthSession(session);
    navigation.navigate('Blank'); await pause(300); await open();
    closes = proof.closes;
    const close = button('Close'); if (!close) throw new Error('Close action absent'); close();
    await wait(() => proof.closes > closes, 'Close action closes'); proof.closeActionClosed = true;
    await wait(() => RN.Dimensions.get('window').height > RN.Dimensions.get('window').width, 'close restores portrait'); proof.portraitRestored = true;
    const attempts = proof.socketAttempts;
    await open({ ...params, nodeId: missingNode });
    await wait(() => proof.phases.at(-1) === 'unavailable', 'real backend refusal');
    await pause(1600);
    proof.refusalVisible = !!button('Retry') && !!button('Close desktop') && proof.socketAttempts === attempts;
    if (!proof.refusalVisible) throw new Error('Backend refusal did not show Retry/Close without automatic retries');
    if (proof.messages.some(value => !['refresh_frame', 'take_control', 'hand_back', 'input'].includes(value.type))) throw new Error('Unexpected desktop application message');
    if (proof.messages.some(value => value.type === 'input' && !value.acknowledged)) throw new Error('Input escaped without server control acknowledgement');
    proof.stage = 'passed';
    return proof;
  } finally { await restore(); delete globalThis.__nyxRelayRestore; }
}

try {
  await writeFile(lock + '/owner', 'B real backend native relay QA ' + new Date().toISOString(), { flag: 'wx' }); ownsLock = true;
  fixture = await relayFixture();
  const pages = await (await fetch('http://localhost:8091/json/list')).json();
  const page = pages.find(value => value.description.startsWith('React Native Bridgeless'));
  assert.ok(page, 'Running iOS development application');
  debuggerSocket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { debuggerSocket.once('open', resolve); debuggerSocket.once('error', reject); });
  let serial = 0;
  evaluate = expression => new Promise((resolve, reject) => {
    const id = ++serial;
    const receive = raw => {
      const response = JSON.parse(raw); if (response.id !== id) return;
      clearTimeout(timer); debuggerSocket.off('message', receive);
      if (response.result?.exceptionDetails) reject(new Error(response.result.exceptionDetails.text));
      else resolve(response.result?.result?.value);
    };
    const timer = setTimeout(() => { debuggerSocket.off('message', receive); reject(new Error('Debugger request deadline')); }, 15000);
    debuggerSocket.on('message', receive);
    debuggerSocket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true } }));
  });
  const expression = require('@babel/core').transformSync(
    'globalThis.__nyxRelayResult=undefined;(' + nativeRelay.toString() + ')(' +
      JSON.stringify(fixture.session) + ',' + JSON.stringify(fixture.params) + ',' + JSON.stringify(randomUUID()) + ')' +
      '.then(value=>{globalThis.__nyxRelayResult={ok:true,value}},error=>{globalThis.__nyxRelayResult={ok:false,error:String(error)}});void 0',
    { configFile: false, babelrc: false, sourceType: 'script', presets: [[require('@react-native/babel-preset'),
      { dev: true, unstable_transformProfile: 'hermes-stable', enableBabelRuntime: false }]] },
  ).code;
  await evaluate(expression);
  const wait = async predicate => {
    const deadline = Date.now() + 120000;
    while (Date.now() < deadline) {
      const result = await evaluate('({proof:globalThis.__nyxRelayProof,result:globalThis.__nyxRelayResult})');
      if (result?.result && !result.result.ok) throw new Error(result.result.error);
      if (predicate(result)) return result;
      await new Promise(resolve => setTimeout(resolve, 200));
    }
    throw new Error('Native relay QA deadline');
  };
  const initial = await wait(value => value?.proof?.stage === 'initial-frames');
  const closes = initial.proof.closes;
  await execute('xcrun', ['simctl', 'launch', device, 'com.apple.Preferences']); appBackgrounded = true;
  await wait(value => value?.proof?.closes > closes);
  await execute('xcrun', ['simctl', 'launch', device, 'dev.nyxid.streama']); appBackgrounded = false;
  const result = (await wait(value => !!value?.result)).result;
  assert.equal(result.ok, true);
  for (const key of ['backgroundClosed', 'foregroundReopened', 'blurClosed', 'epochClosed', 'signOutClosed', 'closeActionClosed', 'refusalVisible', 'explicitHandBack', 'backgroundHandBack', 'landscape', 'portraitRestored']) {
    assert.equal(result.value[key], true, key);
  }
  assert.ok(result.value.decoded.length >= 3 && result.value.decoded.every(value => value.ok && value.accepted), 'All real relay frames decode and composite');
  assert.deepEqual(fixture.metrics.forbiddenOperations, []);
  assert.ok(fixture.metrics.fullFrames >= 2 && fixture.metrics.patchFrames >= 1);
  assert.ok(fixture.metrics.inputs >= 5 && fixture.metrics.handBacks >= 2, 'real node receives fenced input/hand-back');
  assert.equal(fixture.metrics.stops, 0, 'close/background never stops the task');
  console.log(JSON.stringify({ native: result.value, source: fixture.metrics }, null, 2));
  await evaluate('delete globalThis.__nyxRelayProof;delete globalThis.__nyxRelayResult');
} finally {
  try {
    if (appBackgrounded) await execute('xcrun', ['simctl', 'launch', device, 'dev.nyxid.streama']).catch(() => {});
    if (debuggerSocket?.readyState === WebSocket.OPEN && evaluate) {
      await evaluate("if(globalThis.__nyxRelayRestore)globalThis.__nyxRelayRestore().then(()=>{globalThis.__nyxRelayRestored=true});else globalThis.__nyxRelayRestored=true;void 0").catch(() => {});
      const deadline = Date.now() + 10000;
      while (Date.now() < deadline) {
        if (await evaluate('globalThis.__nyxRelayRestored').catch(() => true)) break;
        await new Promise(resolve => setTimeout(resolve, 100));
      }
      await evaluate('delete globalThis.__nyxRelayProof;delete globalThis.__nyxRelayResult;delete globalThis.__nyxRelayRestored').catch(() => {});
    }
    debuggerSocket?.close(); fixture?.close();
  } finally {
    if (ownsLock) { await unlink(lock + '/owner'); await rmdir(lock); }
  }
}
