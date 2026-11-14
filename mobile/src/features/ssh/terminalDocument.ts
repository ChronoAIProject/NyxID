import {
  fitJs,
  jetBrainsMono400,
  xtermCss,
  xtermJs,
} from "./terminalAssets.generated";

// No URL, session, credentials or remote asset enters this document.
export function terminalDocument(_theme?: {
  background: string;
  foreground: string;
  cursor: string;
}) {
  const script = (value: string) => value.replace(/<\/script/gi, "<\\/script");
  const theme = {
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
  return `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; connect-src 'none'; img-src 'none'; font-src data:; media-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'"><style>@font-face{font-family:'JetBrains Mono';src:url(${jetBrainsMono400}) format('truetype');font-weight:400;font-style:normal}${xtermCss}html,body,#terminal{margin:0;width:100%;height:100%;overflow:hidden;background:${theme.background}}</style></head><body><div id="terminal"></div><script>${script(xtermJs)}</script><script>${script(fitJs)}</script><script>
window.open=undefined; window.fetch=undefined; window.WebSocket=undefined; window.XMLHttpRequest=undefined;
document.addEventListener('click',function(e){e.preventDefault();});
const send=(message)=>window.ReactNativeWebView.postMessage(JSON.stringify(message));
const term=new Terminal({fontFamily:"'JetBrains Mono', monospace",fontSize:14,convertEol:false,scrollback:2000,allowProposedApi:false,theme:${JSON.stringify(theme)},linkHandler:{activate(){},hover(){},leave(){}}});
const fit=new FitAddon.FitAddon();term.loadAddon(fit);term.open(document.getElementById('terminal'));
const textarea=document.querySelector('textarea');if(textarea){textarea.setAttribute('autocorrect','off');textarea.setAttribute('autocapitalize','off');textarea.setAttribute('autocomplete','off');textarea.setAttribute('spellcheck','false');}
term.onData((data)=>send({type:'input',data}));
let resizeTimer;function resize(){clearTimeout(resizeTimer);resizeTimer=setTimeout(()=>{try{fit.fit();send({type:'resize',cols:Math.max(10,Math.min(500,term.cols)),rows:Math.max(2,Math.min(200,term.rows))});}catch{}},100);}
const keyInput=(key,application)=>{const arrows={ArrowUp:'A',ArrowDown:'B',ArrowRight:'C',ArrowLeft:'D'};return arrows[key]?'\\x1b'+(application?'O':'[')+arrows[key]:key==='Esc'?'\\x1b':key==='Tab'?'\\t':key;};
window.nyxTerminal=(message)=>{switch(message.type){
case 'output': {const raw=atob(message.data);const data=Uint8Array.from(raw,c=>c.charCodeAt(0));term.write(data,()=>send({type:'ack',id:message.id}));break;}
case 'paste':term.paste(message.text);term.focus();break;
case 'key':term.input(keyInput(message.key,term.modes.applicationCursorKeysMode),true);term.focus();break;
case 'fit':resize();break;
case 'reset':term.reset();term.focus();break;
}};
window.addEventListener('resize',resize);new ResizeObserver(resize).observe(document.getElementById('terminal'));resize();send({type:'ready'});
</script></body></html>`;
}
