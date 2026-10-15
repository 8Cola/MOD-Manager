// Real WebView + independent files. IPC instrumentation records actual calls;
// it does not replace backend results or modify a real game directory.
const fs=require('node:fs/promises'),path=require('node:path'),assert=require('node:assert/strict'),{spawn}=require('node:child_process');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE_PATH||'playwright');
const root=path.resolve(__dirname,'..'),checks=[],timings={};let browser,page,pid,fixture,data;
const delay=ms=>new Promise(r=>setTimeout(r,ms)),quote=s=>"'"+s.replaceAll("'","''")+"'";
async function eventually(test,label,timeout=20000){const end=Date.now()+timeout;while(Date.now()<end){if(await test())return;await delay(80)}throw Error('Timed out: '+label)}
async function ipc(action,args={}){return page.evaluate(async({action,args})=>{const{request}=await import('/src/api.ts');return request(action,args)},{action,args})}
async function stop(){if(browser){await browser.close();browser=null}if(pid){try{process.kill(pid)}catch(e){if(e.code!=='ESRCH')throw e}pid=null}}
async function start(){
 const proc=spawn('powershell.exe',['-NoProfile','-Command',`$env:MM_TEST_DATA_DIR=${quote(data)};$p=Start-Process -FilePath ${quote(path.join(root,require('./dev-paths.cjs').debugExeRelative))} -WindowStyle Hidden -PassThru;Write-Output $p.Id`],{windowsHide:true});
 let out='';proc.stdout.on('data',b=>out+=b);await new Promise((r,j)=>proc.on('exit',c=>c===0?r():j(Error('Launch failed'))));pid=Number(out.trim());
 for(let i=0;i<50;i++){try{browser=await chromium.connectOverCDP('http://127.0.0.1:9222');break}catch{await delay(200)}}assert.ok(browser);page=browser.contexts()[0].pages()[0];await page.locator('.brand').waitFor();
 await page.route('**/node_modules/.vite/deps/chunk-*.js*',async route=>{
  const response=await route.fetch(),source=await response.text();
  const patched=source.replace('return window.__TAURI_INTERNALS__.invoke(cmd, args, options);',"if(cmd==='request')(window.__qaCalls ||= []).push(args.request);return window.__TAURI_INTERNALS__.invoke(cmd,args,options);");
  await route.fulfill({response,body:patched});
 });
 await page.reload();await page.locator('.brand').waitFor();await page.setViewportSize({width:1280,height:850});
}
(async()=>{
 await fs.mkdir(path.join(root,'_local/qa'),{recursive:true});fixture=await fs.mkdtemp(path.join(root,'_local/qa/library-performance-'));data=path.join(fixture,'data');
 const a=path.join(fixture,'A'),b=path.join(fixture,'B');await fs.mkdir(a);await fs.mkdir(b);await start();
 let snapshot=await ipc('saveGame',{game:{name:'Performance QA',strategy:'suffix',folders:[{name:'Regular',path:a},{name:'Shaders',path:b}]}});
 const game=snapshot.games[0];await page.locator('.game-item').first().click();
 // A thousand actual files, not fabricated final UI data.
 await Promise.all(Array.from({length:1000},(_,i)=>fs.writeFile(path.join(a,`sample-${String(i).padStart(4,'0')}.pak`),'fixture')));
 await ipc('scan',{gameId:game.id,folderId:game.folders[0].id});await eventually(async()=>await page.locator('.mod-row').count()===1000,'1000 rows');
 // Let native registration and the file-creation event burst settle before
 // attributing request counts to a later, unrelated button click.
 await delay(5500);
 const search=page.getByRole('textbox',{name:'搜索名称'});const began=Date.now();await search.fill('sample-0999');await eventually(async()=>await page.locator('.mod-row').count()===1,'search result');timings.search1000Ms=Date.now()-began;
 await search.fill('');await eventually(async()=>await page.locator('.mod-row').count()===1000,'search cleared');
 await page.locator('.library-scroll').evaluate(el=>el.scrollTop=el.scrollHeight);await page.locator('.mod-row').last().locator('.mod-open').click();await page.getByRole('dialog',{name:'MOD 详情'}).waitFor();
 checks.push('1000 real MOD files: name search, clearing, scrolling, and opening the last detail remain functional');
 await page.getByRole('textbox',{name:'描述与备注'}).fill('性能验证备注');await page.evaluate(()=>{window.__qaCalls=[]});await page.getByRole('button',{name:'保存信息',exact:true}).click();await page.getByRole('button',{name:'已保存',exact:true}).waitFor();await delay(350);
 let calls=await page.evaluate(()=>window.__qaCalls);assert.equal(calls.filter(c=>c.action==='editMod').length,1);assert.ok(calls.find(c=>c.action==='editMod').compact);assert.equal(calls.filter(c=>['scan','snapshot','viewSnapshot'].includes(c.action)).length,0);
 checks.push('Saving a note makes one compact edit request and no follow-up full scan/snapshot');
 await page.getByRole('dialog',{name:'MOD 详情'}).getByLabel('关闭',{exact:true}).click();await page.getByRole('dialog',{name:'MOD 详情'}).waitFor({state:'hidden'});
 await page.locator('.library-scroll').evaluate(el=>el.scrollTop=0);
 await page.evaluate(()=>{window.__qaCalls=[]});await page.getByRole('button',{name:'刷新目录',exact:true}).click();await eventually(async()=>await page.getByRole('button',{name:'刷新目录',exact:true}).isEnabled(),'refresh done');await delay(250);
 calls=await page.evaluate(()=>window.__qaCalls);const scans=calls.filter(c=>c.action==='scan');assert.equal(scans.length,1);assert.equal(scans[0].folderId,game.folders[0].id);assert.equal(scans[0].compact,true);assert.equal(calls.filter(c=>['snapshot','viewSnapshot'].includes(c.action)).length,0);
 checks.push('Manual directory refresh sends the selected folder, consumes its scoped result, and avoids a second snapshot');
 await fs.writeFile(path.join(b,'shader.pak'),'new externally added file');await eventually(async()=>(await ipc('viewSnapshot')).mods.some(m=>m.folderId===game.folders[1].id),'watcher detects secondary directory');
 checks.push('Native watcher still discovers an externally added MOD in the other directory');
 const directory=path.join(b,'FolderMod');await fs.mkdir(directory);await fs.writeFile(path.join(directory,'inside.pak'),'existing');
 const folders=await ipc('scan',{gameId:game.id,folderId:game.folders[1].id});const folderMod=folders.mods.find(m=>m.members[0].originalPath.endsWith('FolderMod'));
 await ipc('editMod',{id:folderMod.id,managed:true,compact:true});await ipc('toggle',{ids:[folderMod.id],enabled:false});
 assert.equal((await ipc('viewSnapshot')).mods.find(m=>m.id===folderMod.id).state,'disabled');
 await fs.writeFile(path.join(directory,'new.pak'),'keep unchanged');await eventually(async()=>(await ipc('viewSnapshot')).mods.find(m=>m.id===folderMod.id).state==='mixed','disabled folder detects added file');
 assert.equal(await fs.readFile(path.join(directory,'new.pak'),'utf8'),'keep unchanged');assert.equal(await fs.stat(path.join(directory,'new.pak.disabled')).then(()=>true,()=>false),false);
 checks.push('A new file inside a disabled folder becomes mixed through native monitoring; the new file stays unchanged');
 const current=await ipc('viewSnapshot');const small=await ipc('viewSnapshot',{revision:current.revision});assert.equal(small.unchanged,true);assert.ok(JSON.stringify(small).length<1000);checks.push('Unchanged background refresh returns a small acknowledgement instead of 1000 MODs');
 await fs.writeFile(path.join(a,'sample-0000.pak.disabled'),'conflict');const first=current.mods.find(m=>m.members[0].originalPath.endsWith('sample-0000.pak'));
 const result=await ipc('toggle',{ids:[first.id],enabled:false});assert.equal(result.results[0].success,false);assert.equal(await fs.readFile(path.join(a,'sample-0000.pak.disabled'),'utf8'),'conflict');
 checks.push('Cached UI does not bypass toggle preflight: an external filename collision still fails without overwrite');
 await fs.rename(a,a+'-offline');
 try{
  await page.evaluate(()=>{window.__qaCalls=[]});await page.locator('.open-folder').click();await page.getByRole('alert').waitFor();await delay(150);
  calls=await page.evaluate(()=>window.__qaCalls);assert.equal(calls.filter(c=>c.action==='openFolder').length,1);assert.equal(calls.filter(c=>['snapshot','scan','viewSnapshot'].includes(c.action)).length,0);
  assert.equal(await page.getByRole('button',{name:'批量管理',exact:true}).isEnabled(),true);
 }finally{await fs.rename(a+'-offline',a)}
 checks.push('Opening an unavailable folder reports the error without a library refresh or disabling other controls');
 await fs.mkdir(path.join(root,'_local/output'),{recursive:true});await fs.writeFile(path.join(root,'_local/output/library-performance-qa.json'),JSON.stringify({passed:true,checks,timings,fixture},null,2));console.log(JSON.stringify({passed:true,checks,timings},null,2));await stop();
})().catch(async e=>{console.error(e);await stop().catch(()=>{});process.exitCode=1});
