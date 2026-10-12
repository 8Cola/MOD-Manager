const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict'),ts=require('typescript');
const js=ts.transpileModule(fs.readFileSync('src/noteDrafts.ts','utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText;
const testExports={};vm.runInNewContext(js,{exports:testExports,setTimeout,clearTimeout,Promise,Map,Set});const {NoteDrafts}=testExports;
(async()=>{
 let releases=[],writes=[];const store=new NoteDrafts((id,value)=>new Promise(r=>{writes.push(value);releases.push(r)}),()=>{});
 store.read('m','old');store.update('m','A');const save=store.flush('m');store.update('m','B');assert.equal(store.read('m').dirty,true);releases.shift()();await new Promise(r=>setTimeout(r,0));assert.deepEqual(writes,['A','B']);assert.equal(store.read('m').dirty,true);assert.equal(store.read('m').saving,true);releases.shift()();await save;assert.equal(store.read('m').dirty,false);assert.equal(store.read('m').value,'B');
 let failed=true;const retry=new NoteDrafts(async()=>{if(failed)throw Error('disk full')},()=>{});retry.read('m');retry.update('m','keep');await assert.rejects(retry.flush('m'));assert.equal(retry.read('m').dirty,true);assert.equal(retry.read('m').value,'keep');failed=false;await retry.flushAll();assert.equal(retry.read('m').dirty,false);
 retry.update('m','dirty survives pruning');retry.retain(new Set());assert.equal(retry.read('m').value,'dirty survives pruning');await retry.flushAll();retry.retain(new Set());assert.equal(retry.read('m','new record').value,'new record');
 const structural=new NoteDrafts(async()=>{},()=>{});structural.read('old');structural.update('old','latest grouped note');await structural.flushAll();structural.retain(new Set(['new']));assert.equal(structural.read('old','fresh').value,'fresh');await structural.flushAll();
 console.log('PASS: note revision race, queue serialization, failure retention, close flush and retired-ID pruning');
})().catch(e=>{console.error(e);process.exitCode=1});
