import assert from 'node:assert/strict';
import test from 'node:test';
import { guardedJsonTransport } from './guardedTransport';
import { executeAuthenticatedRequest } from './authenticatedTransport';
for(const kind of ['json','stream','binary','voice-live'] as const) for(const retry of [false,true]) for(const refreshed of [false,true]) {
  test(`P1-3 ${kind} retry=${retry} refresh=${refreshed} invalidates only explicit refusal`,async()=>{
    let invalidations=0,refreshes=0;
    const response=()=>Response.json({error:'unauthorized'},{status:401});
    const refresh=async()=>{refreshes++;return refreshed;};
    const invalidate=async()=>{invalidations++;};
    if(kind==='json') await assert.rejects(guardedJsonTransport({send:async()=>response(),read:r=>r.json(),guard:()=>{},refresh,invalidate,failure:()=>new Error('401'),requiresAuth:true,retry}));
    else await assert.rejects(executeAuthenticatedRequest({url:'https://local.test/protected',options:{retryOnAuthFailure:retry,...(kind==='binary'?{method:'POST' as const,rawBody:new ArrayBuffer(1)}:{})},clientHeaders:{},transport:async()=>response(),loadSession:async()=>({accessToken:'a',userId:'person'}),refresh:async()=>(await refresh())?{access_token:'b'}:null,invalidate,toError:()=>new Error('401')}));
    assert.equal(refreshes,retry?1:0);assert.equal(invalidations,retry&&!refreshed?1:0);
  });
}
