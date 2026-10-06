// Only authenticated extension messages can initiate work. The frame rendezvous
// carries a random nonce only; it never transports page data, values or actions.
chrome.runtime.onMessage.addListener((request, sender, respond) => {
  if (sender.id !== chrome.runtime.id || request?.operation !== 'browser') return;
  if (typeof request.nonce !== 'string' || request.expires_at_ms < Date.now()) {
    respond({status: 'refused', reason: 'expired'}); return;
  }
  if (request.action === '_frame_assert') {
    let matches = false;
    try { matches = parent.frames[request.index] === window; } catch { /* detached parent */ }
    respond({status:matches?'ok':'refused'}); return;
  }
  if (request.action === '_frame_signal') {
    parent.postMessage({nyxidFrameProbe: request.probe}, '*');
    respond({status:'ok'}); return;
  }
  if (request.action === '_frame_parent') {
    const timer = setTimeout(() => finish({status:'refused',reason:'frame_not_visible'}), 1000);
    const listener = event => {
      if (event.data?.nyxidFrameProbe !== request.probe) return;
      const frame = Array.from(document.querySelectorAll('iframe,frame')).find(e => e.contentWindow === event.source);
      if (!frame) return;
      try {
        const point = NyxIdBrowser.framePoint(frame, request.point, request.scroll);
        const index = Array.from({length:window.length},(_,i)=>i).find(i=>window.frames[i]===event.source);
        if (index === undefined) throw new Error('frame_not_visible');
        finish({status:'ok',index,point});
      } catch { finish({status:'refused',reason:'overlay_mismatch'}); }
    };
    const finish = result => { clearTimeout(timer); window.removeEventListener('message',listener); respond(result); };
    window.addEventListener('message',listener);
    return true;
  }
  if (request.action === '_visibility') {
    respond({status:'ok',visible:document.visibilityState === 'visible',point:{x:innerWidth/2,y:innerHeight/2,width:innerWidth,height:innerHeight,visible_rect:{left:0,top:0,right:innerWidth,bottom:innerHeight}}}); return;
  }
  NyxIdBrowser.act(request).then(snapshot => respond({status: 'ok', snapshot}), error => {
    const reason = /^(stale_ref|element_disabled|protected_input|input_not_writable|invalid_text|input_refused|invalid_option|key_not_supported|action_not_supported|overlay_mismatch)/.test(error.message)
      ? error.message : 'browser_action_failed';
    respond({status: 'refused', reason});
  });
  return true;
});
