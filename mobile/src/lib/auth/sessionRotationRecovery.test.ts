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

test('M2622-03 failed sign-out retains the pending rotated successor and fences pre-sign-out flights', async () => {
  const e = fixture();
  await e.coordinator.persist(old);
  const captured = (await e.coordinator.load())!;
  e.fail(100);
  await e.coordinator.commitRefresh(captured, next);
  await assert.rejects(e.coordinator.clear(), /locked/);
  assert.equal((await e.coordinator.load())?.refreshToken, 'successor');
  assert.equal(await e.coordinator.commitRefresh(captured, next), false, 'failed sign-out still fences earlier generations');
  e.fail(0);
  const current = (await e.coordinator.load())!;
  assert.equal(current.refreshToken, 'successor');
  assert.equal(JSON.parse(e.values.get(SESSION_RECORD_KEY)!).session.refreshToken, 'successor');
  assert.equal(await e.coordinator.commitRefresh(current, { accessToken: token('a', 2), refreshToken: 'next-successor' }), true);
  await e.coordinator.clear();
  assert.equal(await e.coordinator.load(), null);
});

test('M2622-03 a failed held tombstone cannot restore a successor into a queued replacement login', async () => {
  const e = fixture();
  await e.coordinator.persist(old);
  const captured = (await e.coordinator.load())!;
  e.fail(100);
  await e.coordinator.commitRefresh(captured, next);
  const original = e.storage.setItemAsync;
  let rejectWrite!: () => void;
  e.storage.setItemAsync = async () => new Promise<void>((_resolve, reject) => { rejectWrite = () => reject(Error('locked')); });
  const signOut = assert.rejects(e.coordinator.clear(), /locked/);
  while (!rejectWrite) await new Promise(resolve => setImmediate(resolve));
  const replacement = e.coordinator.persist({ accessToken: token('b', 0), refreshToken: 'b' });
  e.storage.setItemAsync = original;
  e.fail(0);
  rejectWrite();
  await signOut;
  await replacement;
  assert.equal((await e.coordinator.load())?.refreshToken, 'b');
});

test('R1-B Android memory-before-disk tombstone failure keeps an ordinary persisted pair authoritative', async () => {
  const e = fixture();
  await e.coordinator.persist(old);
  const cache = new Map(e.values);
  const durableWrite = e.storage.setItemAsync;
  e.storage.getItemAsync = async key => cache.get(key) ?? null;
  e.storage.setItemAsync = async (key, value) => { cache.set(key, value); throw Error('commit returned false'); };
  await assert.rejects(e.coordinator.clear(), /commit returned false/);
  assert.equal(e.coordinator.peek()?.refreshToken, 'old');
  assert.equal((await e.coordinator.load())?.refreshToken, 'old');
  assert.equal(JSON.parse(e.values.get(SESSION_RECORD_KEY)!).session.refreshToken, 'old');
  e.storage.setItemAsync = async (key, value) => { cache.set(key, value); await durableWrite(key, value); };
  assert.equal((await e.coordinator.load())?.refreshToken, 'old');
  assert.equal((await createSessionCoordinator(e.storage, () => {}).load())?.refreshToken, 'old');
  await e.coordinator.clear();
  assert.equal(await createSessionCoordinator(e.storage, () => {}).load(), null);
});

test('R1-C in-flight refresh survives only failed clears in the exact same session lineage', async () => {
  for (const boundary of ['failed', 'twice-failed', 'success', 'replacement'] as const) {
    const e = fixture(); await e.coordinator.persist(old);
    let sent!: () => void, reply!: (value: any) => void;
    const entered = new Promise<void>(resolve => { sent = resolve; });
    const refresh = createSessionRefresh({load: e.coordinator.load, commit: e.coordinator.commitRefresh, expiresAt: n => n,
      request: async value => { assert.equal(value, 'old'); sent(); return new Promise(resolve => { reply = resolve; }); }});
    const flight = refresh();
    const result = flight.then(() => 'accepted', error => error.name);
    await entered;
    if (boundary === 'success') await e.coordinator.clear();
    else {
      e.fail(100); await assert.rejects(e.coordinator.clear(), /locked/);
      if (boundary === 'twice-failed') await assert.rejects(e.coordinator.clear(), /locked/);
      e.fail(0);
      if (boundary === 'replacement') await e.coordinator.persist({accessToken: token('b', 0), refreshToken: 'b'});
    }
    reply({access_token: next.accessToken, refresh_token: next.refreshToken, expires_in: 60});
    assert.equal(await result, boundary === 'failed' || boundary === 'twice-failed' ? 'accepted' : 'AbortError', boundary);
    const loaded = await e.coordinator.load();
    assert.equal(loaded?.refreshToken ?? null, boundary === 'success' ? null : boundary === 'replacement' ? 'b' : 'successor', boundary);
    if (boundary === 'failed' || boundary === 'twice-failed') {
      assert.equal(JSON.parse(e.values.get(SESSION_RECORD_KEY)!).session.refreshToken, 'successor');
      assert.equal(await e.coordinator.commitRefresh({...loaded!, refreshToken: 'wrong'}, next), false);
      assert.equal(await e.coordinator.commitRefresh(loaded!, {accessToken: token('b', 2), refreshToken: 'foreign'}), false);
    }
  }
});
