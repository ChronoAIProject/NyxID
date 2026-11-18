import assert from 'node:assert/strict';
import test from 'node:test';
import { createSessionCoordinator, SESSION_RECORD_KEY, type SessionStorage } from './sessionCoordinator';
import { createSessionRefresh } from './sessionRefresh';
const token=(sub:string,n:number)=>`e30.${Buffer.from(JSON.stringify({sub,n})).toString('base64url')}.sig`;
const old={accessToken:token('a',0),refreshToken:'old'};
const next={accessToken:token('a',1),refreshToken:'successor'};
function fixture() {
  const values=new Map<string,string>(); let failures=0, writes=0;
  const storage:SessionStorage={getItemAsync:async k=>values.get(k)??null,deleteItemAsync:async k=>{values.delete(k);},setItemAsync:async(k,v)=>{writes++;if(failures-->0)throw Error('locked');values.set(k,v);}};
  const coordinator=createSessionCoordinator(storage,()=>{});
  return {coordinator,values,storage,fail:(n:number)=>{failures=n;writes=0;},writes:()=>writes};
}
test('P4 retries bounded immediate failures and durably saves the rotated successor',async()=>{
  const e=fixture();await e.coordinator.persist(old);const before=(await e.coordinator.load())!;
  e.fail(2);assert.equal(await e.coordinator.commitRefresh(before,next),true);assert.equal(e.writes(),3);
  assert.equal(JSON.parse(e.values.get(SESSION_RECORD_KEY)!).session.refreshToken,'successor');
});
test('P4 memory successor survives unreadable storage and later loads retry persisting it',async()=>{
  const e=fixture();await e.coordinator.persist(old);const before=(await e.coordinator.load())!;e.fail(100);
  assert.equal(await e.coordinator.commitRefresh(before,next),true);assert.equal(e.writes(),3);
  e.storage.getItemAsync=async()=>{throw Error('unreadable');};
  assert.equal((await e.coordinator.load())?.refreshToken,'successor');assert.ok(e.writes()<=6);
  e.fail(0);assert.equal((await e.coordinator.load())?.refreshToken,'successor');
  assert.equal(JSON.parse(e.values.get(SESSION_RECORD_KEY)!).session.refreshToken,'successor');
});
test('P4 later refresh never resends the retired token even if persistence keeps failing',async()=>{
  const e=fixture();await e.coordinator.persist(old);const used:string[]=[];
  const refresh=createSessionRefresh({load:e.coordinator.load,commit:e.coordinator.commitRefresh,expiresAt:v=>v,request:async refreshToken=>{used.push(refreshToken);return {access_token:token('a',used.length),refresh_token:`successor-${used.length}`,expires_in:60};}});
  e.fail(100);assert.ok(await refresh());assert.ok(await refresh());assert.deepEqual(used,['old','successor-1']);
});
test('P4 explicit login or sign-out discards a pending in-memory successor',async()=>{
  for(const action of ['login','logout']) {
    const e=fixture();await e.coordinator.persist(old);const before=(await e.coordinator.load())!;e.fail(100);
    await e.coordinator.commitRefresh(before,next);e.fail(0);
    if(action==='login')await e.coordinator.persist({accessToken:token('b',0),refreshToken:'b'});else await e.coordinator.clear();
    assert.equal((await e.coordinator.load())?.refreshToken??null,action==='login'?'b':null);
  }
});
test('P4 an auth replacement during a held persist fences the in-memory successor',async()=>{
  const e=fixture();await e.coordinator.persist(old);const before=(await e.coordinator.load())!;
  let release!:()=>void;const original=e.storage.setItemAsync;
  e.storage.setItemAsync=async()=>{await new Promise<void>(r=>{release=r;});throw Error('locked');};
  const rotation=e.coordinator.commitRefresh(before,next);
  while(!release)await new Promise(r=>setImmediate(r));
  const logout=e.coordinator.clear();e.storage.setItemAsync=original;release();
  assert.equal(await rotation,false);await logout;assert.equal(await e.coordinator.load(),null);
});

test('N6 later loads make one persistence attempt while commit retains its three-attempt burst', async () => {
  const e=fixture();await e.coordinator.persist(old);const before=(await e.coordinator.load())!;
  e.fail(100);await e.coordinator.commitRefresh(before,next);assert.equal(e.writes(),3);
  for(let load=1;load<=5;load++) {
    assert.equal((await e.coordinator.load())?.refreshToken,'successor');
    assert.equal(e.writes(),3+load,'one later native write per load');
  }
  e.fail(0);await e.coordinator.load();assert.equal(e.writes(),1);
  assert.equal(JSON.parse(e.values.get(SESSION_RECORD_KEY)!).session.refreshToken,'successor');
});

test('M2622-01 a successful response never loses its successor to a post-response native read fault', async () => {
  const e=fixture(); await e.coordinator.persist(old);
  const originalRead=e.storage.getItemAsync; let failRead=false; const used:string[]=[];
  e.storage.getItemAsync=async key => {if(failRead) {failRead=false;throw Error('temporary read');}return originalRead(key);};
  const refresh=createSessionRefresh({load:e.coordinator.load,commit:e.coordinator.commitRefresh,expiresAt:v=>v,request:async value=>{
    used.push(value); failRead=true;
    return {access_token:token('a',used.length),refresh_token:`successor-${used.length}`,expires_in:60};
  }});
  assert.ok(await refresh());
  // The fault is still armed: commit performed no fallible post-response read.
  await assert.rejects(e.coordinator.load(), /temporary read/);
  assert.equal((await e.coordinator.load())?.refreshToken,'successor-1');
  assert.ok(await refresh()); assert.deepEqual(used,['old','successor-1']);
  assert.equal(JSON.parse(e.values.get(SESSION_RECORD_KEY)!).session.refreshToken,'successor-2');
});
test('M2622-01 known predecessor and subject fencing rejects mismatches and replacement flights', async () => {
  const e=fixture();await e.coordinator.persist(old);const captured=(await e.coordinator.load())!;
  const original=e.storage.getItemAsync;e.storage.getItemAsync=async()=>{throw Error('must not read');};
  assert.equal(await e.coordinator.commitRefresh({...captured,refreshToken:'wrong'},next),false);
  assert.equal(await e.coordinator.commitRefresh(captured,{accessToken:token('b',1),refreshToken:'other'}),false);
  e.storage.getItemAsync=original;
  await e.coordinator.persist({accessToken:token('b',0),refreshToken:'b'});
  assert.equal(await e.coordinator.commitRefresh(captured,next),false);
  assert.equal((await e.coordinator.load())?.refreshToken,'b');
});
