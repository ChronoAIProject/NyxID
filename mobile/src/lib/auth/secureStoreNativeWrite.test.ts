import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { createSessionCoordinator, SESSION_RECORD_KEY } from './sessionCoordinator';
const require = createRequire(import.meta.url);
const root=path.dirname(require.resolve('expo-secure-store/package.json'));
test('M2622-09 installed pinned SecureStore rejects a false encrypted SharedPreferences commit before legacy cleanup',()=>{
  assert.equal(JSON.parse(readFileSync(path.join(root,'package.json'),'utf8')).version,'55.0.12');
  const app=JSON.parse(readFileSync(new URL('../../../package.json',import.meta.url),'utf8'));
  assert.ok(app.expo.autolinking.android.buildFromSource?.includes('expo-secure-store'), 'patched Android Kotlin must be compiled instead of using the bundled Maven AAR');
  const source=readFileSync(path.join(root,'android/src/main/java/expo/modules/securestore/SecureStoreModule.kt'),'utf8');
  const write=source.slice(source.indexOf('private suspend fun setItemImpl'),source.indexOf('private fun saveEncryptedItem'));
  assert.match(write,/if \(!saveEncryptedItem\(encryptedItem, prefs, keychainAwareKey, options.requireAuthentication, options.keychainService\)\) \{\s*throw WriteException\("Could not write encrypted value to SecureStore", key, options.keychainService\)\s*\}/);
  assert.ok(write.indexOf('if (!saveEncryptedItem') < write.indexOf('prefs.edit().remove(key).apply()'));
  assert.match(source,/return prefs.edit\(\).putString\(key, encryptedItemString\).commit\(\)/);
});
test('M2622-09 native Android write rejection retains successor until the next durable commit',async()=>{
  const values=new Map<string,string>();let failures=0;
  const storage={getItemAsync:async(k:string)=>values.get(k)??null,deleteItemAsync:async(k:string)=>{values.delete(k);},setItemAsync:async(k:string,v:string)=>{
    if(failures-->0)throw Object.assign(Error('Could not write encrypted value to SecureStore'),{code:'ERR_SECURESTORE_WRITE_ERROR'});
    values.set(k,v);
  }};
  const run=createSessionCoordinator(storage,()=>{});
  const access=(n:number)=>`e30.${Buffer.from(JSON.stringify({sub:'a',n})).toString('base64url')}.sig`;
  await run.persist({accessToken:access(0),refreshToken:'predecessor'});const prior=(await run.load())!;
  failures=4;assert.equal(await run.commitRefresh(prior,{accessToken:access(1),refreshToken:'successor'}),true);
  assert.equal(JSON.parse(values.get(SESSION_RECORD_KEY)!).session.refreshToken,'predecessor');
  assert.equal((await run.load())?.refreshToken,'successor');
  assert.equal((await run.load())?.refreshToken,'successor');
  assert.equal(JSON.parse(values.get(SESSION_RECORD_KEY)!).session.refreshToken,'successor');
});
