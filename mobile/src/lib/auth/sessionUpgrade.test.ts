import assert from 'node:assert/strict';
import test from 'node:test';
import { createSessionCoordinator, type SessionStorage, type StoredAuthSession } from './sessionCoordinator';
const token = (sub: string, label: string) => `e30.${Buffer.from(JSON.stringify({sub,label})).toString('base64url')}.sig`;
const a = {accessToken:token('a','old'),refreshToken:'a-old',userId:'a',accessTokenExpiresAt:100};
const b = {accessToken:token('b','new'),refreshToken:'b-new',userId:'b',accessTokenExpiresAt:200};
const rotated = {...a,accessToken:token('a','new'),refreshToken:'a-new',accessTokenExpiresAt:300};
const prefix = 'nyxid.auth.';
function legacy() { return new Map(Object.entries({access_token:a.accessToken,refresh_token:a.refreshToken,user_id:'a',access_token_expires_at:'100'}).map(([k,v])=>[prefix+k,v])); }
function fixture(values: Map<string,string>, interruptAt=Infinity) {
  let count=0;
  const step=()=>{if(++count===interruptAt) throw new Error('simulated process interruption');};
  const storage:SessionStorage={
    getItemAsync:async key=>{const value=values.get(key)??null;step();return value;},
    setItemAsync:async(key,value)=>{values.set(key,value);step();},
    deleteItemAsync:async key=>{values.delete(key);step();},
  };
  return {coordinator:createSessionCoordinator(storage,()=>{}),count:()=>count};
}
function usable(session:StoredAuthSession|null,expected= a) {assert.equal(session?.accessToken,expected.accessToken);assert.equal(session?.refreshToken,expected.refreshToken);assert.equal(session?.userId,expected.userId);}
test('shared iOS/Android storage protocol survives interruption after every migration storage operation',async()=>{
  const measured=fixture(legacy());await measured.coordinator.load();
  for(let boundary=1;boundary<=measured.count();boundary++){
    const values=legacy();const run=fixture(values,boundary);await run.coordinator.load().catch(()=>{});
    usable(await fixture(values).coordinator.load());
  }
});
test('login replacement never publishes mixed accounts after interruption at any operation',async()=>{
  const seed=legacy();await fixture(seed).coordinator.load();
  const measured=fixture(new Map(seed));await measured.coordinator.persist(b);
  for(let boundary=1;boundary<=measured.count();boundary++){
    const values=new Map(seed);await fixture(values,boundary).coordinator.persist(b).catch(()=>{});
    const restored=await fixture(values).coordinator.load();assert.ok(restored);
    usable(restored,restored.accessToken===a.accessToken?a:b);
  }
});
test('rotation leaves a usable old-access/new-refresh or complete successor after every operation',async()=>{
  const seed=legacy();await fixture(seed).coordinator.load();
  const baseline=fixture(new Map(seed));const captured=(await baseline.coordinator.load())!;
  const before=baseline.count();await baseline.coordinator.commitRefresh(captured,rotated);const operations=baseline.count()-before;
  for(let boundary=1;boundary<=operations;boundary++){
    const values=new Map(seed);const capture=(await fixture(values).coordinator.load())!;
    await fixture(values,boundary).coordinator.commitRefresh(capture,rotated).catch(()=>{});
    const restored=await fixture(values).coordinator.load();assert.ok(restored);
    assert.ok([a.accessToken,rotated.accessToken].includes(restored.accessToken));
    assert.ok([a.refreshToken,rotated.refreshToken].includes(restored.refreshToken!));assert.equal(restored.userId,'a');
  }
});
test('legacy copies are removed only after the complete fresh v2 record exists; signout removes secrets and retains the authoritative nonsecret tombstone',async()=>{
  const values=legacy();const run=fixture(values);await run.coordinator.load();
  assert.ok([...values.keys()].some(k=>k.startsWith(prefix+'v2.')),'fresh key must apply current Keychain accessibility');
  assert.ok(!values.has(prefix+'refresh_token'));await run.coordinator.clear();assert.equal(values.size,1);assert.deepEqual(JSON.parse(values.get(prefix+'v2.session')!),{version:2,session:null});
});

test('M2622-02 fulfilled native deletions cannot resurrect legacy credentials after sign-out',async()=>{
  const values=legacy();
  const storage:SessionStorage={getItemAsync:async k=>values.get(k)??null,setItemAsync:async(k,v)=>{values.set(k,v);},deleteItemAsync:async k=>{if(k.includes('.v2.'))values.delete(k);}};
  const run=createSessionCoordinator(storage,()=>{});
  usable(await run.load());await run.clear();
  assert.ok(values.has(prefix+'access_token'),'native legacy deletion silently failed');
  assert.deepEqual(JSON.parse(values.get(prefix+'v2.session')!),{version:2,session:null});
  for(let boot=0;boot<3;boot++)assert.equal(await createSessionCoordinator(storage,()=>{}).load(),null);
  const login=createSessionCoordinator(storage,()=>{});await login.persist(b);
  usable(await createSessionCoordinator(storage,()=>{}).load(),b);
});
