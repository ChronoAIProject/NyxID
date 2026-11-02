import { useEffect, useRef, useState } from 'react';
import { Alert, AppState, Keyboard, KeyboardAvoidingView, Platform, StatusBar, StyleSheet, Text, TextInput, View } from 'react-native';
import { useIsFocused } from '@react-navigation/native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { WebView } from 'react-native-webview';
import { ActionButton, Notice } from '../../../components/workspace/WorkspaceUI';
import { useTheme } from '../../../theme/ThemeContext';
import { lockDesktopLandscape, restorePortrait } from '../../../lib/desktopOrientation';
import { getAuthEpoch } from '../../../lib/auth/authEpoch';
import { loadStoredAuthSession } from '../../../lib/auth/sessionStore';
import { refreshAccessTokenIfNeeded, requestJson } from '../../../lib/api/http';
import { getApiBaseUrl } from '../../../lib/api/baseUrl';
import { canvasDocument } from './canvasDocument';
import { createCompositor } from './compositor';
import { desktopSocketUrl, type MachineDesktopParams } from './desktopRoutes';
import { createDesktopViewer, refusalLabel, type ViewerState, type ViewSocket } from './viewer';

export type { MachineDesktopParams } from './desktopRoutes';
export type MachineDesktopScreenProps = {
  route: { params: MachineDesktopParams };
  navigation: { goBack(): void };
  /** Host must protect native app-switcher snapshots before mounting. */
  privacyProtected?: boolean;
};
const documentSource = { html: canvasDocument(), baseUrl: 'about:blank' };


/** Root full-screen route. Control intent lives in the socket owner, never in
 * renderer messages. Native snapshot protection is required in addition to JS. */
export function MachineDesktopScreen({ route, navigation, privacyProtected = false }: MachineDesktopScreenProps) {
  const focused = useIsFocused();
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const { nodeId, conversationId, contextId, display = 'secure' } = route.params;
  const webView = useRef<WebView>(null);
  const privacyCover = useRef<View>(null);
  const [foreground, setForeground] = useState(AppState.currentState === 'active');
  const [oriented, setOriented] = useState(false);
  const [ready, setReady] = useState(false);
  const [retry, setRetry] = useState(0);
  const [hasPixels, setHasPixels] = useState(false);
  const [state, setState] = useState<ViewerState>({ phase: 'connecting', label: 'Connecting to the desktop…' });
  const [overlay, setOverlay] = useState(true);
  const [keyboard, setKeyboard] = useState(false);
  const [draft, setDraft] = useState('');
  const [inputError, setInputError] = useState<string>();
  const viewerEpoch = useRef(-1);
  const viewer = useRef<ReturnType<typeof createDesktopViewer>>(undefined);
  const decodeTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const overlayTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const decoding = useRef<{ generation: number; sequence: string }>(undefined);
  const pendingRead = useRef<AbortController>(undefined);
  const compositor = useRef<ReturnType<typeof createCompositor>>(undefined);
  const clear = () => {
    pendingRead.current?.abort(); pendingRead.current = undefined;
    clearTimeout(decodeTimer.current); decoding.current = undefined;
    privacyCover.current?.setNativeProps({ style: { opacity: 1 } });
    setHasPixels(false); setKeyboard(false); setDraft(''); setInputError(undefined); Keyboard.dismiss();
    const generation = compositor.current?.reset() ?? 0;
    webView.current?.injectJavaScript(`window.nyxDesktop?.clear(${generation});true;`);
  };
  const stop = () => { viewer.current?.stop(); viewer.current = undefined; clear(); };
  const close = () => { stop(); void restorePortrait().catch(() => {}); navigation.goBack(); };
  const revealOverlay = () => {
    setOverlay(true); clearTimeout(overlayTimer.current);
    overlayTimer.current = setTimeout(() => setOverlay(false), 4500);
  };

  useEffect(() => {
    const subscription = AppState.addEventListener('change', next => {
      if (next !== 'active') { stop(); setReady(false); setOriented(false); void restorePortrait().catch(() => {}); }
      setForeground(next === 'active');
    });
    return () => { subscription.remove(); stop(); clearTimeout(overlayTimer.current); void restorePortrait().catch(() => {}); };
  }, []);

  useEffect(() => {
    let current = true;
    if (!focused || !foreground) {
      stop(); setOriented(false); void restorePortrait().catch(() => {});
      return;
    }
    void lockDesktopLandscape().then(() => { if (current) setOriented(true); }).catch(error => {
      if (current) { stop(); setState({ phase: 'unavailable', label: refusalLabel(error) }); }
    });
    return () => { current = false; stop(); setOriented(false); void restorePortrait().catch(() => {}); };
  }, [focused, foreground, retry]);

  useEffect(() => {
    if (!privacyProtected || !focused || !foreground || !oriented || !ready) { stop(); return; }
    const epoch = getAuthEpoch(); viewerEpoch.current = epoch;
    let mounted = true;
    const current = () => mounted && getAuthEpoch() === epoch && AppState.currentState === 'active';
    compositor.current = createCompositor(frame => {
      if (!current()) return;
      decoding.current = { generation: frame.generation, sequence: frame.sequence };
      webView.current?.injectJavaScript(`window.nyxDesktop?.frame(${JSON.stringify(frame)});true;`);
      clearTimeout(decodeTimer.current);
      decodeTimer.current = setTimeout(() => viewer.current?.fail('The desktop renderer stopped responding.'), 8000);
    }, () => viewer.current?.refresh());
    const activeViewer = createDesktopViewer({
      current, clear, state: value => {
        if (!current()) return;
        setState(value);
        webView.current?.injectJavaScript(`window.nyxDesktop?.control(${value.controls === true});true;`);
        if (!value.controls) { setKeyboard(false); setDraft(''); setInputError(undefined); Keyboard.dismiss(); }
        revealOverlay();
      },
      invalidated: () => {
        if (mounted && getAuthEpoch() !== epoch) {
          void restorePortrait().catch(() => {});
          navigation.goBack();
        }
      },
      frame: frame => compositor.current?.enqueue(frame),
      connect: async () => {
        const read = new AbortController(); pendingRead.current = read;
        await refreshAccessTokenIfNeeded();
        if (!current()) throw new Error('Your sign-in session changed.');
        await requestJson(`/assistant/nyxagent/machines/${encodeURIComponent(nodeId)}/contexts`, {
          invalidateSessionOnAuthFailure: false, signal: read.signal,
          authorityGuard: () => { if (!current()) throw new Error('Your sign-in session changed.'); },
        });
        const session = await loadStoredAuthSession();
        if (!current() || !session?.accessToken) throw Object.assign(new Error('Sign in to open this desktop.'), { statusCode: 401 });
        const NativeWebSocket = WebSocket as unknown as {
          new(url: string, protocols: string[], options: { headers: Record<string, string> }): ViewSocket;
        };
        return new NativeWebSocket(desktopSocketUrl(getApiBaseUrl(), { nodeId, conversationId, contextId, display }), [], { headers: { Authorization: `Bearer ${session.accessToken}` } });
      },
    });
    viewer.current = activeViewer; clear(); activeViewer.start();
    return () => { mounted = false; activeViewer.stop(); clear(); };
  }, [privacyProtected, focused, foreground, oriented, ready, retry, nodeId, conversationId, contextId, display]);

  const button = (label: string, action: () => void, disabled = false, longPress?: () => void) => <ActionButton
    key={label} label={label} variant="secondary" disabled={disabled}
    accessibilityHint={longPress ? "Touch and hold for task stop options" : undefined}
    onPress={() => { revealOverlay(); action(); }} onLongPress={longPress} />;
  const sendText = () => {
    if (!draft) return;
    if (viewer.current?.input({ tool: 'type_text', arguments: { text: draft } })) {
      setDraft(''); setInputError(undefined);
    } else {
      setInputError('Check control and try again in a moment, or send a shorter message.');
    }
  };
  const controlOptions = () => {
    if (!state.controls) return;
    Alert.alert('Desktop control', 'Stopping the task cancels its running machine work.', [
      { text: 'Cancel', style: 'cancel' },
      { text: 'Stop task', style: 'destructive', onPress: () => viewer.current?.stopTask() },
    ]);
  };
  const unavailable = !privacyProtected || state.phase === 'unavailable';
  const visible = overlay || keyboard || unavailable || !hasPixels || state.taking;
  return <KeyboardAvoidingView style={styles.screen} behavior={Platform.OS === 'ios' ? 'padding' : undefined} onAccessibilityEscape={close}>
    <StatusBar hidden />
    {focused && foreground && <WebView ref={webView} source={documentSource} style={styles.webView}
      originWhitelist={['about:blank']} onShouldStartLoadWithRequest={request => request.url === 'about:blank'}
      javaScriptEnabled domStorageEnabled={false} cacheEnabled={false} incognito sharedCookiesEnabled={false}
      thirdPartyCookiesEnabled={false} allowFileAccess={false} allowFileAccessFromFileURLs={false}
      allowUniversalAccessFromFileURLs={false} javaScriptCanOpenWindowsAutomatically={false}
      setSupportMultipleWindows={false} allowsLinkPreview={false} allowsBackForwardNavigationGestures={false}
      scrollEnabled={false} bounces={false} textZoom={100} automaticallyAdjustContentInsets={false}
      onLoadStart={() => { stop(); setReady(false); }}
      onContentProcessDidTerminate={() => { stop(); setReady(false); setState({ phase: 'unavailable', label: 'The desktop renderer ended.' }); }}
      onMessage={event => {
        if (event.nativeEvent.data.length > 20_000) return;
        try {
          const value = JSON.parse(event.nativeEvent.data) as Record<string, unknown>;
          if (value.type === 'ready') { setReady(true); return; }
          if (value.type === 'interaction') { revealOverlay(); return; }
          if (value.type === 'toggleOverlay') { setOverlay(previous => !previous); return; }
          if (value.type === 'input') { viewer.current?.input(value.input); return; }
          if (value.type === 'unavailable') { stop(); setState({ phase: 'unavailable', label: 'This device cannot render the desktop.' }); return; }
          if (value.type !== 'decoded' || typeof value.generation !== 'number' || typeof value.sequence !== 'string' || typeof value.ok !== 'boolean') return;
          if (decoding.current?.generation !== value.generation || decoding.current.sequence !== value.sequence) return;
          clearTimeout(decodeTimer.current); decoding.current = undefined;
          const accepted = compositor.current?.acknowledge({ generation: value.generation, sequence: value.sequence, ok: value.ok });
          if (accepted && viewer.current && getAuthEpoch() === viewerEpoch.current && focused && AppState.currentState === 'active') {
            privacyCover.current?.setNativeProps({ style: { opacity: 0 } }); setHasPixels(true);
          }
        } catch { viewer.current?.fail('The desktop renderer returned unsupported data.'); }
      }} />}
    <View ref={privacyCover} pointerEvents="none" accessibilityElementsHidden importantForAccessibility="no-hide-descendants"
      style={[StyleSheet.absoluteFillObject, styles.cover, { opacity: hasPixels && focused && foreground ? 0 : 1 }]} />
    {visible && <View style={[styles.overlay, { top: Math.max(8, insets.top), left: Math.max(8, insets.left), right: Math.max(8, insets.right) }]}>
      {button('Close', close)}
      {button(state.controls ? 'Hand back' : state.taking ? 'Taking control…' : 'Take control', () => state.controls ? viewer.current?.handBack() : viewer.current?.takeControl(), unavailable || !hasPixels || state.taking, controlOptions)}
      {button('Keyboard', () => setKeyboard(value => !value), !state.controls)}
      {button('Refresh', () => viewer.current?.refresh(), unavailable)}
    </View>}
    {(unavailable || !hasPixels || state.phase === 'attention') && <View pointerEvents="box-none" style={styles.messageWrap}>
      <Text accessibilityLiveRegion="polite" style={styles.message}>{privacyProtected ? state.label : 'Desktop snapshot protection is unavailable on this device.'}</Text>
      {unavailable && <View style={styles.retryRow}>{button('Retry', () => { stop(); setReady(false); webView.current?.reload(); setState({ phase: 'connecting', label: 'Connecting to the desktop…' }); setRetry(value => value + 1); })}{button('Close desktop', close)}</View>}
    </View>}
    {keyboard && state.controls && <View style={[styles.keyboard, { backgroundColor: colors.card, bottom: Math.max(8, insets.bottom), left: Math.max(8, insets.left), right: Math.max(8, insets.right) }]}>
      {inputError && <Notice message={inputError} tone="error" />}
      <View style={styles.keyRow}>
        {(['ENTER', 'TAB', 'BACKSPACE', 'ESC'] as const).map(key => button(key === 'BACKSPACE' ? '⌫' : key, () => viewer.current?.input({ tool: 'press_key', arguments: { key } })))}
        {button('Hide keyboard', () => { setKeyboard(false); Keyboard.dismiss(); })}
      </View>
      <View style={styles.keyRow}><TextInput autoFocus value={draft} onChangeText={setDraft} maxLength={8192}
        accessibilityLabel="Desktop text to send" placeholder="Type text, then Send" placeholderTextColor="#a1a1aa" style={styles.textInput}
        autoCorrect={false} autoCapitalize="none" secureTextEntry onSubmitEditing={sendText} />{button('Send', sendText, !draft)}</View>
    </View>}
    {!visible && <View style={styles.reveal}><ActionButton label="•••" accessibilityLabel="Show desktop controls" variant="secondary" onPress={revealOverlay} /></View>}
  </KeyboardAvoidingView>;
}
const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: '#000' }, webView: { flex: 1, backgroundColor: '#000' },
  cover: { backgroundColor: '#000' }, overlay: { position: 'absolute', flexDirection: 'row', flexWrap: 'wrap', gap: 8 },
  messageWrap: { position: 'absolute', bottom: 24, left: 24, right: 24, alignItems: 'center' },
  message: { backgroundColor: '#18181be8', color: '#fafafa', padding: 12, borderRadius: 8, textAlign: 'center', fontSize: 15 },
  retryRow: { flexDirection: 'row', flexWrap: 'wrap', gap: 8, paddingTop: 12 },
  keyboard: { position: 'absolute', padding: 8, borderRadius: 12, gap: 8 }, keyRow: { flexDirection: 'row', gap: 8, flexWrap: 'wrap' },
  textInput: { flex: 1, minWidth: 120, color: '#fafafa', fontSize: 17, padding: 12, backgroundColor: '#27272a', borderRadius: 8 },
  reveal: { position: 'absolute', top: 8, left: 8, minWidth: 44 },
});
