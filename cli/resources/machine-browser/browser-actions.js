/* Fixed browser operations shared by the extension and the dev-browser isolated
 * world. The secure browser never evaluates caller-supplied JavaScript. */
(() => {
  const refs = new WeakMap(), elements = new Map();
  const documentId = crypto.randomUUID();
  let nextRef = 1;
  const selector = 'a[href],button,input,textarea,select,[role="button"],[role="link"],[role="textbox"],[contenteditable="true"],[tabindex]';
  const secret = element => element instanceof HTMLInputElement &&
    (element.type === 'password' || /(?:^|\s)(?:current-password|new-password|one-time-code)(?:\s|$)/i.test(element.autocomplete));
  const visible = element => {
    const style = getComputedStyle(element);
    return !!element.getClientRects().length && style.visibility !== 'hidden' && style.display !== 'none' && !element.closest('[inert],[aria-hidden="true"]');
  };
  const clip = (value, max = 100) => String(value || '').replace(/\s+/g, ' ').trim().slice(0, max);
  function label(element) {
    return clip(element.getAttribute('aria-label') ||
      (element.getAttribute('aria-labelledby') || '').split(/\s+/).map(id => document.getElementById(id)?.textContent || '').join(' ') ||
      Array.from(element.labels || []).map(node => { const copy = node.cloneNode(true); copy.querySelectorAll('input,textarea,select').forEach(control => control.remove()); return copy.textContent; }).join(' ') ||
      element.getAttribute('placeholder') || element.getAttribute('title') ||
      (element.matches('input,textarea,select') ? element.name || element.id : element.innerText));
  }
  function ref(element) {
    if (!refs.has(element)) refs.set(element, `${documentId}.${nextRef++}`);
    const value = refs.get(element);
    elements.set(value, element);
    return value;
  }
  function snapshot(request = {}) {
    for (const [id, element] of elements) if (!element.isConnected) elements.delete(id);
    const root = request.scope ? target(request.scope) : document.body || document.documentElement;
    const query = typeof request.query === 'string' ? {text: request.query} : request.query || {};
    const contains = (value, part) => !part || String(value).toLowerCase().includes(String(part).toLowerCase());
    const role = element => element.getAttribute('role') ||
      (element instanceof HTMLInputElement ? ({checkbox:'checkbox',radio:'radio',range:'slider',number:'spinbutton',button:'button',submit:'button',reset:'button'}[element.type] || 'textbox') :
       ({a:'link',textarea:'textbox',select:'combobox'}[element.tagName.toLowerCase()] || element.tagName.toLowerCase()));
    const matches = element => contains(role(element), query.role) && contains(label(element), query.label) &&
      contains(`${role(element)} ${label(element)} ${element.matches('input,textarea,select') ? '' : clip(element.innerText, 600)}`, query.text);
    const all = Array.from(root.querySelectorAll(selector)).filter(e => visible(e) && matches(e));
    const offset = Math.max(0, Math.min(100000, Number(request.offset) || 0));
    const interactive = all.slice(offset, offset + 60).map(element => ({
      ref: ref(element), role: role(element), label: label(element),
      ...(element instanceof HTMLInputElement ? {kind: element.type, protected: secret(element)} : {}),
      ...(element.disabled ? {disabled: true} : {}),
      ...(element instanceof HTMLSelectElement ? {options: Array.from(element.options).slice(0, 12).map(option => ({label: clip(option.label), value: clip(option.value)}))} : {}),
    }));
    let text = '', textMore = false;
    const walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let node; (node = walk.nextNode());) {
      const parent = node.parentElement;
      if (parent && !parent.closest('script,style,noscript,input,textarea,select,[contenteditable="true"]') && visible(parent) && contains(node.textContent, query.text)) {
        const next = clip(node.textContent, 400);
        if (text.length + next.length > 1400) { textMore = true; break; }
        text += `${next} `;
      }
    }
    const result = {url: location.href.slice(0,512), title: clip(document.title, 160),
      headings: Array.from(root.querySelectorAll('h1,h2,h3,[role="heading"]')).filter(visible).filter(e => contains(e.innerText, query.text)).slice(0,12).map(e => clip(e.innerText)),
      text: clip(text, 1400), elements: interactive, ready: document.readyState,
      offset, total: all.length, more: {elements: offset + interactive.length < all.length, text: textMore}};
    while (new TextEncoder().encode(JSON.stringify(result)).length > 5700 && result.elements.length) result.elements.pop();
    while (new TextEncoder().encode(JSON.stringify(result)).length > 5700 && result.text.length) {
      result.text = result.text.slice(0, Math.max(0, result.text.length - 200)); result.more.text = true;
    }
    while (new TextEncoder().encode(JSON.stringify(result)).length > 5700 && result.headings.length) result.headings.pop();
    result.more.elements = offset + result.elements.length < all.length;
    if (result.more.elements) result.next_offset = offset + result.elements.length;
    return result;
  }
  function geometry(element, scroll = true) {
    if (scroll) element.scrollIntoView({block:'center', inline:'center', behavior:'instant'});
    const rect = element.getBoundingClientRect();
    const x = Math.max(0,rect.left) + (Math.min(innerWidth,rect.right)-Math.max(0,rect.left))/2;
    const y = Math.max(0,rect.top) + (Math.min(innerHeight,rect.bottom)-Math.max(0,rect.top))/2;
    const hit = document.elementFromPoint(x,y);
    if (x < 0 || y < 0 || x >= innerWidth || y >= innerHeight || !(hit === element || element.contains(hit))) throw new Error('overlay_mismatch: target is covered; observe before retrying');
    return {x,y,width:innerWidth,height:innerHeight,dpr:devicePixelRatio,screen_x:screenX,screen_y:screenY,
      ui_x:(outerWidth-innerWidth)/2,ui_y:outerHeight-innerHeight-(outerWidth-innerWidth)/2,scroll_x:scrollX,scroll_y:scrollY};
  }
  function prepare(request) {
    const action = request.input_action;
    const element = request.ref ? target(request.ref) : document.activeElement;
    if (!element || !visible(element)) throw new Error('stale_ref');
    if (action === 'type') {
      if (secret(element)) throw new Error('protected_input: use fill_login for credentials');
      if ((!element.matches('input,textarea') && !element.isContentEditable) || element.readOnly ||
          element instanceof HTMLInputElement && !['text','email','tel','url','search','number'].includes(element.type)) throw new Error('input_not_writable');
      if (typeof request.text !== 'string' || request.text.length > 16000) throw new Error('invalid_text');
    }
    if (action === 'press') {
      if (!['Enter','Tab','Escape','ArrowUp','ArrowDown','ArrowLeft','ArrowRight','Home','End','PageUp','PageDown','Backspace','Delete'].includes(request.key)) throw new Error('key_not_supported');
      if (secret(element) && !['Tab','Enter','Escape'].includes(request.key)) throw new Error('protected_input');
    }
    if (['type','press','select'].includes(action)) {
      element.focus({preventScroll:true});
      if (document.activeElement !== element) throw new Error('input_not_writable: target cannot receive keys');
    }
    let option;
    if (action === 'select') {
      if (!(element instanceof HTMLSelectElement)) throw new Error('invalid_option');
      option = Array.from(element.options).findIndex(o => o.value === request.value && !o.disabled);
      if (option < 0) throw new Error('invalid_option');
    }
    return {point:geometry(element), focused:document.activeElement === element, ...(option !== undefined ? {option} : {})};
  }
  function target(id) {
    const element = elements.get(id);
    if (!element?.isConnected || !visible(element)) throw new Error('stale_ref: take a new snapshot');
    if (element.disabled) throw new Error('element_disabled');
    return element;
  }
  const settle = () => new Promise(resolve => setTimeout(resolve, 35));
  // Layout and the compositor can lag a new iframe document. Focus explicitly
  // and wait for presentation before handing coordinates to trusted OS input.
  const painted = () => new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('input_not_writable: page is not ready for input')), 1000);
    requestAnimationFrame(() => requestAnimationFrame(() => {clearTimeout(timer);resolve();}));
  });
  async function act(request) {
    if (['click','type','select','press'].includes(request.action)) prepare({...request,input_action:request.action});
    switch (request.action) {
      case "_prepare": prepare(request); await painted(); return prepare(request);
      case "_visibility": return {x:innerWidth/2,y:innerHeight/2,width:innerWidth,height:innerHeight,visible_rect:{left:0,top:0,right:innerWidth,bottom:innerHeight}};
      case 'snapshot': break;
      case 'click': target(request.ref).click(); break;
      case 'type': {
        const element = target(request.ref);
        if (secret(element)) throw new Error('protected_input: use fill_login for credentials');
        if ((!element.matches('input,textarea') && !element.isContentEditable) || element.readOnly ||
            element instanceof HTMLInputElement && !['text','email','tel','url','search','number'].includes(element.type)) throw new Error('input_not_writable');
        if (typeof request.text !== 'string' || request.text.length > 16000) throw new Error('invalid_text');
        element.focus();
        if (element.select) element.select();
        else { const range = document.createRange(); range.selectNodeContents(element); const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range); }
        if (!document.execCommand('insertText', false, request.text)) throw new Error('input_refused');
        element.dispatchEvent(new Event('change', {bubbles: true}));
        break;
      }
      case 'select': {
        const element = target(request.ref);
        if (!(element instanceof HTMLSelectElement) || typeof request.value !== 'string' || !Array.from(element.options).some(o => o.value === request.value && !o.disabled)) throw new Error('invalid_option');
        element.value = request.value;
        element.dispatchEvent(new Event('input', {bubbles: true}));
        element.dispatchEvent(new Event('change', {bubbles: true}));
        break;
      }
      case 'press': {
        const element = request.ref ? target(request.ref) : document.activeElement;
        const key = request.key;
        if (!['Enter','Tab','Escape','ArrowUp','ArrowDown','ArrowLeft','ArrowRight','Home','End','PageUp','PageDown','Backspace','Delete'].includes(key)) throw new Error('key_not_supported');
        if (secret(element) && !['Tab','Enter','Escape'].includes(key)) throw new Error('protected_input: use fill_login for credentials');
        element?.focus();
        if (key === 'Tab') {
          const order = Array.from(document.querySelectorAll(selector)).filter(e => visible(e) && !e.disabled && e.tabIndex >= 0);
          order[(order.indexOf(element) + 1) % order.length]?.focus();
        } else {
          const proceed = element?.dispatchEvent(new KeyboardEvent('keydown', {key, bubbles: true, cancelable: true}));
          if (proceed && key === 'Enter' && element?.form) element.form.requestSubmit();
          else if (proceed && key === 'Enter' && element?.matches('button,a')) element.click();
          else if (proceed && ['PageUp','PageDown','Home','End'].includes(key) && !element?.matches('input,textarea')) {
            if (key === 'Home' || key === 'End') window.scrollTo(0, key === 'Home' ? 0 : document.documentElement.scrollHeight);
            else window.scrollBy(0, innerHeight * (key === 'PageUp' ? -0.8 : 0.8));
          } else if (proceed && element?.matches('input,textarea') && !secret(element)) {
            const start = element.selectionStart, end = element.selectionEnd;
            if (start !== null && end !== null) {
              if (key === 'Backspace' || key === 'Delete') {
                element.setSelectionRange(start === end && key === 'Backspace' ? Math.max(0,start-1) : start,
                  start === end && key === 'Delete' ? end+1 : end);
                document.execCommand('delete');
              } else if (['ArrowLeft','ArrowRight','Home','End'].includes(key)) {
                const at = key === 'Home' ? 0 : key === 'End' ? element.textLength : Math.max(0, start + (key === 'ArrowLeft' ? -1 : 1));
                element.setSelectionRange(at, at);
              }
            }
          }
          element?.dispatchEvent(new KeyboardEvent('keyup', {key, bubbles: true}));
        }
        break;
      }
      case 'scroll': window.scrollBy({left: Math.max(-4000, Math.min(4000, Number(request.x) || 0)), top: Math.max(-4000, Math.min(4000, Number(request.y) || 0)), behavior: 'instant'}); break;
      case 'find': {
        if (typeof request.text !== 'string' || !request.text || request.text.length > 500) throw new Error('invalid_text');
        const matches = Array.from(document.querySelectorAll('p,li,td,th,h1,h2,h3,label,button,a,span')).filter(e => visible(e) && !e.closest('input,textarea,select') && e.innerText.includes(request.text));
        matches[0]?.scrollIntoView({block: 'center'});
        return {...snapshot(request), matches: matches.slice(0, 10).map(e => ({ref: ref(e), text: clip(e.innerText, 240)}))};
      }
      default: throw new Error('action_not_supported');
    }
    if (request.action !== 'snapshot') await settle();
    return snapshot(request);
  }
  function framePoint(frame, point, scroll = false) {
    if (scroll) frame.scrollIntoView({block:'nearest',inline:'nearest',behavior:'instant'});
    const rect=frame.getBoundingClientRect(), sx=rect.width/(frame.offsetWidth||rect.width),sy=rect.height/(frame.offsetHeight||rect.height);
    const mapX = x => rect.left+(frame.clientLeft+x*frame.clientWidth/point.width)*sx;
    const mapY = y => rect.top+(frame.clientTop+y*frame.clientHeight/point.height)*sy;
    let x=mapX(point.x),y=mapY(point.y),visible_rect;
    if (point.visible_rect) {
      // A tall iframe can be visible while its centre lies below the viewport.
      // Propagate its visible intersection through every ancestor for snapshots.
      const v=point.visible_rect;
      visible_rect={left:Math.max(0,mapX(v.left)),top:Math.max(0,mapY(v.top)),
        right:Math.min(innerWidth,mapX(v.right)),bottom:Math.min(innerHeight,mapY(v.bottom))};
      if (visible_rect.right<=visible_rect.left || visible_rect.bottom<=visible_rect.top) throw new Error('overlay_mismatch');
      x=(visible_rect.left+visible_rect.right)/2;y=(visible_rect.top+visible_rect.bottom)/2;
    }
    if (document.elementFromPoint(x,y)!==frame) throw new Error('overlay_mismatch');
    return {...point,x,y,...(visible_rect?{visible_rect}:{}),width:innerWidth,height:innerHeight,dpr:devicePixelRatio,screen_x:screenX,screen_y:screenY,
      ui_x:(outerWidth-innerWidth)/2,ui_y:outerHeight-innerHeight-(outerWidth-innerWidth)/2};
  }
  // Isolated-world global only; the page has no bridge to this object.
  globalThis.NyxIdBrowser = {act, snapshot, secret, geometry, framePoint};
})();
