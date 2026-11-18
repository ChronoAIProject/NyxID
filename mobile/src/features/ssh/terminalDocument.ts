import {
  fitJs,
  jetBrainsMono400,
  xtermCss,
  xtermJs,
} from "./terminalAssets.generated";

export const TERMINAL_THEME = {
  background: "#0f172a",
  foreground: "#e2e8f0",
  cursor: "#c76a34",
  selectionBackground: "#334155",
  selectionForeground: "#f8fafc",
  black: "#1e293b",
  red: "#ef4444",
  green: "#22c55e",
  yellow: "#eab308",
  blue: "#3b82f6",
  magenta: "#a855f7",
  cyan: "#06b6d4",
  white: "#e2e8f0",
  brightBlack: "#475569",
  brightRed: "#f87171",
  brightGreen: "#4ade80",
  brightYellow: "#facc15",
  brightBlue: "#60a5fa",
  brightMagenta: "#c084fc",
  brightCyan: "#22d3ee",
  brightWhite: "#f8fafc",
};

export const terminalBridgeScript = `
window.open=undefined; window.fetch=undefined; window.WebSocket=undefined; window.XMLHttpRequest=undefined;
document.addEventListener('click',function(e){e.preventDefault();});
const send=(message)=>window.ReactNativeWebView.postMessage(JSON.stringify(message));
const term=new Terminal({fontFamily:"'JetBrains Mono', monospace",fontSize:14,convertEol:false,scrollback:2000,allowProposedApi:false,theme:${JSON.stringify(TERMINAL_THEME)},linkHandler:{activate(){},hover(){},leave(){}}});
const fit=new FitAddon.FitAddon();term.loadAddon(fit);term.open(document.getElementById('terminal'));
const textarea=document.querySelector('textarea');if(textarea){textarea.setAttribute('autocorrect','off');textarea.setAttribute('autocapitalize','off');textarea.setAttribute('autocomplete','off');textarea.setAttribute('spellcheck','false');}
let interactive=true;
const endSession=(message)=>{interactive=false;term.options.disableStdin=true;term.options.cursorBlink=false;if(textarea)textarea.disabled=true;term.write('\\r\\n\\x1b[0m\\x1b[2m'+String(message).replace(/[\\u0000-\\u001f\\u007f-\\u009f]/g,c=>c==='\\n'?'\\r\\n':'')+'\\x1b[0m\\r\\n\\x1b[?25l');term.blur();};
term.onData((data)=>{if(interactive)send({type:'input',data});});
let resizeTimer;function resize(){clearTimeout(resizeTimer);resizeTimer=setTimeout(()=>{try{fit.fit();send({type:'resize',cols:Math.max(10,Math.min(500,term.cols)),rows:Math.max(2,Math.min(200,term.rows))});}catch{}},100);}
const keyInput=(key,application)=>{const arrows={ArrowUp:'A',ArrowDown:'B',ArrowRight:'C',ArrowLeft:'D'};return arrows[key]?'\\x1b'+(application?'O':'[')+arrows[key]:key==='Esc'?'\\x1b':key==='Tab'?'\\t':key;};
window.nyxTerminal=(message)=>{switch(message.type){
case 'output': {const raw=atob(message.data);const data=Uint8Array.from(raw,c=>c.charCodeAt(0));term.write(data,()=>send({type:'ack',id:message.id}));break;}
case 'paste':if(interactive){term.paste(message.text);term.focus();}break;
case 'key':if(interactive){term.input(keyInput(message.key,term.modes.applicationCursorKeysMode),true);term.focus();}break;
case 'fit':resize();break;
case 'session-ended':endSession(message.message);break;
case 'reset':interactive=true;term.options.disableStdin=false;if(textarea)textarea.disabled=false;term.reset();term.write('\\x1b[?25h');term.focus();break;
}};
window.addEventListener('resize',resize);new ResizeObserver(resize).observe(document.getElementById('terminal'));resize();send({type:'ready'});
`;

export function escapeInlineScript(value: string): string {
  let escaped = "";
  let from = 0;
  for (
    let start = value.indexOf("</");
    start !== -1;
    start = value.indexOf("</", from)
  ) {
    const prefix = value.slice(start, start + 8).toLowerCase();
    escaped += value.slice(from, start) + (prefix === "</script" ? "<\\/" : "</");
    from = start + 2;
  }
  return escaped + value.slice(from);
}

// No URL, session, credentials or remote asset enters this document.
export function terminalDocument(_theme?: {
  background: string;
  foreground: string;
  cursor: string;
}) {
  return `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; connect-src 'none'; img-src 'none'; font-src data:; media-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'"><style>@font-face{font-family:'JetBrains Mono';src:url(${jetBrainsMono400}) format('truetype');font-weight:400;font-style:normal}${xtermCss}html,body,#terminal{margin:0;width:100%;height:100%;overflow:hidden;background:${TERMINAL_THEME.background}}</style></head><body><div id="terminal"></div><script>${escapeInlineScript(xtermJs)}</script><script>${escapeInlineScript(fitJs)}</script><script>${terminalBridgeScript}</script></body></html>`;
}
