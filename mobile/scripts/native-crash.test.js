const test=require('node:test');
const assert=require('node:assert/strict');
const {execFileSync}=require('node:child_process');
const {mkdtempSync,rmSync}=require('node:fs');
const {tmpdir}=require('node:os');
const path=require('node:path');
test('Swift decision drops mismatched/no binding and RN SIGABRT duplicate; sanitizers never emit private fields',()=>{
 const directory=mkdtempSync(path.join(tmpdir(),'nyx-crash-swift-'));
 try {
  const source=path.resolve(__dirname,'../modules/nyx-crash-reporter');
  const binary=path.join(directory,'proof');
  execFileSync('swiftc',[path.join(source,'ios/CrashDecision.swift'),path.join(source,'tests/main.swift'),'-o',binary],{stdio:'pipe'});
  assert.match(execFileSync(binary,{encoding:'utf8'}),/proofs passed/);
 } finally {rmSync(directory,{recursive:true,force:true});}
});
