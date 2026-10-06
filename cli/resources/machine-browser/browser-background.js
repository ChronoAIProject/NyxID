/* Supervisor-only native requests. No externally-connectable or page bridge. */
const NyxIdBrowserBackground = (() => {
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  function url(value) {
    const parsed = new URL(value);
    if (!['http:', 'https:'].includes(parsed.protocol) || parsed.username || parsed.password) throw new Error('invalid_url');
    return parsed.href;
  }
  const parsedRef = ref => {
    const match = /^f(\d+):(.+)$/.exec(ref || '');
    return match ? {frame:Number(match[1]),ref:match[2]} : {frame:0,ref};
  };
  async function send(tab, frame, request) {
    return chrome.tabs.sendMessage(tab.id, {...request,operation:'browser'}, {frameId:frame});
  }
  async function topPoint(tab, frames, frameId, point, request, scroll) {
    let child = frames.find(f => f.frameId === frameId);
    for (let depth=0; child?.parentFrameId >= 0 && depth<16; depth++) {
      const probe = crypto.randomUUID();
      const pending = send(tab,child.parentFrameId,{...request,action:'_frame_parent',point,probe,scroll});
      // Chrome delivers sendMessage listeners in order, before the child's signal.
      await pause(5);
      await send(tab,child.frameId,{...request,action:'_frame_signal',probe});
      const result = await pending;
      if (result?.status !== 'ok') throw new Error(result?.reason || 'frame_not_visible');
      // A page can observe/replay postMessage nonces. Ask the authenticated
      // child isolated world to verify the mapped parent slot before trusting it.
      const verified = await send(tab,child.frameId,{...request,action:'_frame_assert',index:result.index});
      if (verified?.status !== 'ok') throw new Error('frame_identity_mismatch');
      point=result.point;
      child=frames.find(f => f.frameId===child.parentFrameId);
    }
    if (!child || child.frameId !== 0) throw new Error('frame_not_visible');
    return point;
  }
  async function snapshot(tab, request, frames) {
    const scope = request.scope ? parsedRef(request.scope) : null;
    const candidates = frames.filter(f => !scope || f.frameId === scope.frame);
    const pages = [];
    const offset = Math.max(0, Math.min(100000, Number(request.offset) || 0));
    let skip = offset, total = 0;
    for (const frame of candidates.slice(0,64)) {
      try {
        if (frame.frameId !== 0) {
          const visible = await send(tab, frame.frameId, {...request, action:'_visibility'});
          if (!visible?.visible) continue;
          await topPoint(tab, frames, frame.frameId, visible.point, request, false);
        }
        const response = await send(tab, frame.frameId, {...request, action:'snapshot', offset:skip, scope:scope?.ref});
        if (!response?.snapshot) continue;
        const page = response.snapshot;
        total += page.total || 0; skip = Math.max(0, skip - (page.total || 0));
        pages.push({frame:frame.frameId,page});
      } catch { /* inaccessible, hidden, detached or covered child frame */ }
    }
    const result = {url:tab.url?.slice(0,512),title:tab.title?.slice(0,160),headings:[],text:'',elements:[],frames:[],ready:'complete',offset,total,more:{elements:false,text:false,frames:candidates.length>64}};
    const size = value => new TextEncoder().encode(JSON.stringify(value)).length;
    // Reserve bounded space for frame/truncation metadata before adding rows.
    for (const {frame,page} of pages) {
      if (frame === 0) {result.url=page.url;result.title=page.title;result.ready=page.ready;}
      const metadata={id:`f${frame}`,more:{elements:!!page.more?.elements,text:!!page.more?.text}};
      if (size([...result.frames,metadata])<=1200) result.frames.push(metadata);
      else result.more.frames=true;
      const text=page.text||'';
      result.more.text ||= !!page.more?.text || result.text.length+text.length>1400;
      result.text+=(result.text?' ':'')+text.slice(0,Math.max(0,1400-result.text.length));
      for (const heading of page.headings||[]) {
        if (size([...result.headings,heading])<=600) result.headings.push(heading);
      }
    }
    while (size(result)>4000 && result.text.length) {result.text=result.text.slice(0,-200);result.more.text=true;}
    let full=false;
    for (const {frame,page} of pages) {
      const metadata=result.frames.find(f=>f.id===`f${frame}`);
      for (const element of page.elements||[]) {
        if (full) {if(metadata)metadata.more.elements=true;break;}
        result.elements.push({...element,ref:`f${frame}:${element.ref}`});
        if (result.elements.length>60 || size(result)>6400) {
          result.elements.pop();full=true;if(metadata)metadata.more.elements=true;break;
        }
      }
      // Pagination is contiguous across frames. Do not skip a truncated tail
      // and then advance next_offset past elements we never returned.
      if (page.more?.elements) full=true;
    }
    result.more.elements=offset+result.elements.length<total;
    if (result.more.elements) result.next_offset=offset+result.elements.length;
    return result;
  }
  async function perform(request) {
    const {action}=request;
    let tabs=await chrome.tabs.query({});
    let tab=request.tab_id ? tabs.find(t=>String(t.id)===request.tab_id) : (await chrome.tabs.query({active:true,lastFocusedWindow:true}))[0];
    if (request.tab_id && !tab) return {status:'refused',reason:'tab_not_found'};
    if (action==='tabs_new') tab=await chrome.tabs.create({url:url(request.url||'https://example.com'),active:true});
    if (!tab) return {status:'refused',reason:'tab_not_found'};
    switch(action) {
      case 'navigate':tab=await chrome.tabs.update(tab.id,{url:url(request.url)});break;
      case 'back':await chrome.tabs.goBack(tab.id);break;
      case 'forward':await chrome.tabs.goForward(tab.id);break;
      case 'tabs_switch':await chrome.tabs.update(tab.id,{active:true});await chrome.windows.update(tab.windowId,{focused:true});break;
      case 'tabs_close':await chrome.tabs.remove(tab.id);tab=(await chrome.tabs.query({active:true,lastFocusedWindow:true}))[0];break;
    }
    if (!tab) return {status:'ok',snapshot:{tabs:[]}};
    const frames=await chrome.webNavigation.getAllFrames({tabId:tab.id})||[];
    const target=parsedRef(request.ref);
    if (action==='_prepare') {
      await chrome.tabs.update(tab.id,{active:true});
      await chrome.windows.update(tab.windowId,{focused:true});
      const response=await send(tab,target.frame,{...request,ref:target.ref});
      if (response?.status!=='ok') return response;
      try {
        const prepared=response.snapshot;
        prepared.point=await topPoint(tab,frames,target.frame,prepared.point,request,true);
        return {status:'ready',tab_id:String(tab.id),...prepared};
      } catch {return {status:'refused',reason:'overlay_mismatch'};}
    }
    const navigation=['navigate','back','forward','tabs_new','tabs_switch','tabs_close'].includes(action);
    let effect;
    if (!navigation && !['tabs','wait','snapshot'].includes(action)) {
      try {effect=await send(tab,target.frame,{...request,ref:target.ref});}
      catch { /* Never replay an uncertain action. */ }
      if (effect?.status==='refused') return effect;
    }
    const deadline=Date.now()+Math.max(100,Math.min(15000,Number(request.timeout_ms)||5000));
    let page;
    do {
      try {
        tab=await chrome.tabs.get(tab.id);
        if (tab.status==='loading'||tab.pendingUrl) {await pause(35);continue;}
        page=await snapshot(tab,request,await chrome.webNavigation.getAllFrames({tabId:tab.id})||[]);
        if (page && (action!=='wait'||(!request.text||page.text.includes(request.text))&&page.ready==='complete')) break;
      } catch { /* Observe the new document after navigation. */ }
      await pause(35);
    } while(Date.now()<deadline);
    tabs=await chrome.tabs.query({});
    const tabRows=[];
    for (const t of tabs.slice().sort((a,b)=>Number(b.id===tab.id)-Number(a.id===tab.id))) {
      const row={id:String(t.id),title:t.title?.slice(0,80),url:t.url?.slice(0,240),active:t.active};
      if (new TextEncoder().encode(JSON.stringify([...tabRows,row])).length>1500) break;
      tabRows.push(row);
    }
    return {status:page?'ok':'refused',...(!page?{reason:'page_not_ready: retry snapshot after navigation'}:{}),
      snapshot:{...(page||{url:tab.url?.slice(0,512),title:tab.title?.slice(0,160)}),tab_id:String(tab.id),tabs:tabRows},
      ...(action==='wait'?{matched:!!page&&page.ready==='complete'&&(!request.text||page.text.includes(request.text))}:{}),
      ...(['click','type','select','press'].includes(action)?{input_mode:'dom_fallback'}:{})};
  }
  return {perform};
})();
