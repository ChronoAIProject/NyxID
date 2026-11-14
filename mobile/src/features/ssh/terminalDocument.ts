import { fitJs, xtermCss, xtermJs } from "./terminalAssets.generated";

// No URL, session, credentials or remote asset enters this document.
export function terminalDocument(theme: {
  background: string;
  foreground: string;
  cursor: string;
}) {
  const script = (value: string) => value.replace(/<\/script/gi, "<\\/script");
  return `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; connect-src 'none'; img-src 'none'; font-src 'none'; media-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'"><style>${xtermCss}html,body,#terminal{margin:0;width:100%;height:100%;overflow:hidden;background:${theme.background}}</style></head><body><div id="terminal"></div><script>${script(xtermJs)}</script><script>${script(fitJs)}</script><script>
window.open=undefined; window.fetch=undefined; window.WebSocket=undefined; window.XMLHttpRequest=undefined;
document.addEventListener('click',function(e){e.preventDefault();});
const send=(message)=>window.ReactNativeWebView.postMessage(JSON.stringify(message));
const term=new Terminal({fontFamily:"ui-monospace, Menlo, 'Roboto Mono', monospace",fontSize:14,convertEol:false,scrollback:2000,allowProposedApi:false,theme:${JSON.stringify(theme)},linkHandler:{activate(){},hover(){},leave(){}}});
const fit=new FitAddon.FitAddon();term.loadAddon(fit);term.open(document.getElementById('terminal'));
const textarea=document.querySelector('textarea');if(textarea){textarea.autocorrect='off';textarea.autocapitalize='off';textarea.spellcheck=false;textarea.setAttribute('autocomplete','off');}
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
