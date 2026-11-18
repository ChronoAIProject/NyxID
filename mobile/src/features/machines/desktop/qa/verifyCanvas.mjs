import { createRequire } from 'node:module';
import { performance } from 'node:perf_hooks';
import assert from 'node:assert/strict';
import { canvasDocument } from '../canvasDocument.ts';
import { desktopFrame } from '../protocol.ts';
import { RED_JPEG, GREEN_PATCH, BLUE_JPEG, SESSION, vector } from '../testVectors.ts';
const require = createRequire(new URL('../../../../../../frontend/package.json', import.meta.url));
const { chromium } = require('@playwright/test');
const browser = await chromium.launch({headless:true});
try {
 const page=await browser.newPage();const messages=[];const requests=[];const errors=[];
 await page.exposeFunction('bridge',text=>messages.push(JSON.parse(text)));
 await page.addInitScript(()=>window.ReactNativeWebView={postMessage:value=>window.bridge(value)});
 page.on('request',req=>{if(!req.url().startsWith('blob:')&&!req.url().startsWith('data:'))requests.push(req.url());});
 page.on('pageerror',error=>errors.push(error.message));
 // setContent preserves the local, network-free document, like source.html.
 await page.goto('about:blank');
 await page.evaluate(()=>{window.ReactNativeWebView={postMessage:value=>window.bridge(value)};window.__urlCount=0;window.__peakUrls=0;const create=URL.createObjectURL.bind(URL),revoke=URL.revokeObjectURL.bind(URL);URL.createObjectURL=value=>{window.__urlCount++;window.__peakUrls=Math.max(window.__peakUrls,window.__urlCount);return create(value);};URL.revokeObjectURL=value=>{window.__urlCount--;return revoke(value);};});
 await page.setContent(canvasDocument());
 await page.waitForFunction(()=>typeof window.nyxDesktop?.clear === "function");
 await page.evaluate(()=>window.nyxDesktop.clear(1));
 const send=async(jpeg,sequence,rectangle)=>{
  const size=desktopFrame(vector(new Uint8Array(Buffer.from(jpeg,'base64')),BigInt(sequence),rectangle&&{...rectangle,patchWidth:2,patchHeight:2,base:BigInt(rectangle.base)}),SESSION);
  assert.ok(size);
  const before=messages.length;
  await page.evaluate(f=>window.nyxDesktop.frame(f),{generation:1,sequence:String(sequence),jpeg,width:size.width,height:size.height,rectangle});
  for(let i=0;i<100 && messages.length===before;i++)await page.waitForTimeout(10);
  assert.equal(messages.at(-1)?.ok,true);
 };
 const pixel=async(x,y)=>page.evaluate(([x,y])=>Array.from(document.querySelector('canvas').getContext('2d').getImageData(x,y,1,1).data),[x,y]);
 await send(RED_JPEG,1);const red=await pixel(0,0);assert.ok(red[0]>240 && red[1]<15 && red[2]<15);
 await send(GREEN_PATCH,2,{width:8,height:6,x:2,y:2,base:'1'});
 const green=await pixel(2,2);assert.ok(green[1]>240 && green[0]<15 && green[2]<15);assert.deepEqual(await pixel(0,0),red);
 await send(BLUE_JPEG,3);const blue=await pixel(0,0);assert.ok(blue[2]>240 && blue[0]<15 && blue[1]<15);
 const reset=await page.evaluate(()=>({width:document.querySelector('canvas').width,height:document.querySelector('canvas').height}));assert.equal(reset.width,4);
 const before=messages.length;
 await page.evaluate(jpeg=>window.nyxDesktop.frame({generation:1,sequence:'4',jpeg,width:1920,height:1200}),GREEN_PATCH);
 for(let i=0;i<100 && messages.length===before;i++)await page.waitForTimeout(10);
 assert.equal(messages.at(-1)?.ok,false,'actual decoded dimensions checked');
 const corrupt=Buffer.from([255,216,255,192,0,17,8,0,6,0,8,3,1,17,0,2,17,0,3,17,0,255,217]).toString('base64');
 const corruptBefore=messages.length;await page.evaluate(jpeg=>window.nyxDesktop.frame({generation:1,sequence:'4',jpeg,width:8,height:6}),corrupt);
 for(let n=0;n<100&&messages.length===corruptBefore;n++)await page.waitForTimeout(10);
 assert.equal(messages.at(-1)?.ok,false,'corrupt JPEG rejected by actual decoder');
 assert.deepEqual(await pixel(0,0),blue,'decode failures preserve the last valid surface');
 // Maximum bridge copy and true decoded full-frame cost: synthetic 1920x1200 JPEG.
 const large=await page.evaluate(()=>{const c=document.createElement('canvas');c.width=1920;c.height=1200;const x=c.getContext('2d');const pixels=x.createImageData(1920,1200);let seed=17;for(let i=0;i<pixels.data.length;i+=4){seed=(seed*1664525+1013904223)>>>0;pixels.data[i]=seed&255;pixels.data[i+1]=(seed>>>8)&255;pixels.data[i+2]=(seed>>>16)&255;pixels.data[i+3]=255;}x.putImageData(pixels,0,0);return c.toDataURL('image/jpeg',0.9).split(',')[1];});
 const cdp=await page.context().newCDPSession(page);
 await cdp.send('HeapProfiler.collectGarbage');const heapBefore=await cdp.send('Runtime.getHeapUsage');
 const timings=[];
 for(let i=0;i<20;i++){const before=messages.length,start=performance.now();await page.evaluate(jpeg=>window.nyxDesktop.frame({generation:1,sequence:'5',jpeg,width:1920,height:1200}),large);for(let n=0;n<100&&messages.length===before;n++)await page.waitForTimeout(1);assert.equal(messages.at(-1)?.ok,true);timings.push(performance.now()-start);}
 const beforeCancel=messages.length;await page.evaluate(jpeg=>{window.nyxDesktop.frame({generation:1,sequence:'6',jpeg,width:1920,height:1200});window.nyxDesktop.clear(2);},large);await page.waitForTimeout(50);assert.equal(messages.length,beforeCancel,'cancelled decode cannot acknowledge or repaint');assert.deepEqual(await page.evaluate(()=>[document.querySelector('canvas').width,document.querySelector('canvas').height]),[0,0]);
 assert.deepEqual(errors,[]);assert.deepEqual(requests,[]);
 const urls=await page.evaluate(()=>({active:window.__urlCount,peak:window.__peakUrls}));assert.deepEqual(urls,{active:0,peak:1});
 await cdp.send('HeapProfiler.collectGarbage');const heapAfter=await cdp.send('Runtime.getHeapUsage');
 const result={fullRed:red,patchGreen:green,fullResetBlue:blue,fullResetSize:reset,frames:20,largeJpegBytes:Buffer.from(large,'base64').length,bridgeCharacters:large.length,meanBridgeDecodeMs:timings.reduce((a,b)=>a+b)/timings.length,maxBridgeDecodeMs:Math.max(...timings),networkRequests:requests,blobUrls:urls,heapBefore,heapAfter,errors};
 console.log(JSON.stringify(result,null,2));
} finally {await browser.close();}
