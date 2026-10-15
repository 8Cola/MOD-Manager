const fs=require('node:fs/promises'),path=require('node:path'),assert=require('node:assert/strict'),{spawn}=require('node:child_process');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE_PATH||'playwright');
const root=path.resolve(__dirname,'..'),checks=[];let browser,page,pid,fixture,data;
const delay=ms=>new Promise(r=>setTimeout(r,ms)),quote=s=>"'"+s.replaceAll("'","''")+"'";
async function start(){const proc=spawn('powershell.exe',['-NoProfile','-Command',`$env:MM_TEST_DATA_DIR=${quote(data)};$p=Start-Process -FilePath ${quote(path.join(root,require('./dev-paths.cjs').debugExeRelative))} -WindowStyle Hidden -PassThru;Write-Output $p.Id`],{windowsHide:true});let out='';proc.stdout.on('data',b=>out+=b);await new Promise((r,j)=>proc.on('exit',c=>c===0?r():j(Error('launch failed'))));pid=Number(out.trim());for(let i=0;i<40;i++){try{browser=await chromium.connectOverCDP('http://127.0.0.1:9222');break}catch{await delay(500)}}assert.ok(browser);page=browser.contexts()[0].pages()[0];await page.locator('.brand').waitFor();await page.setViewportSize({width:1240,height:820});await preparePicker();}
async function stop(){if(browser){await browser.close();browser=null}if(pid){try{process.kill(pid)}catch(e){if(e.code!=='ESRCH')throw e}pid=null}await delay(200)}
async function ipc(action,args={}){return page.evaluate(async({action,args})=>{const api=await import('/src/api.ts');return api.request(action,args)},{action,args});}
async function eventually(test,label,timeout=15000){const until=Date.now()+timeout;while(Date.now()<until){const result=await test();if(result)return result;await delay(80)}throw Error('Timed out: '+label)}
async function interceptPicker(file){await page.evaluate(file=>{window.__qaPick=file},file)}
async function preparePicker(){await page.route('**/*plugin-dialog*.js*',async route=>{const response=await route.fetch();const source=await response.text();const patched=source.replace(/return await invoke\w*\(["']plugin:dialog\|open["'], \{\s*options\s*\}\);/, 'return window.__qaPick;');assert.notEqual(patched,source,'picker fixture module route applied');await route.fulfill({response,body:patched});});await page.reload();await page.locator('.brand').waitFor();}
async function decodeImage(file){return page.evaluate(async file=>{const {request}=await import('/src/api.ts'),{url}=await request('previewImage',{path:file});const im=new Image();im.src=url;await im.decode();const c=document.createElement('canvas');c.width=im.width;c.height=im.height;const ctx=c.getContext('2d');ctx.drawImage(im,0,0);return {width:im.width,height:im.height,center:[...ctx.getImageData(im.width/2,im.height/2,1,1).data]}},file)}
async function drag(type,paths){await page.evaluate(async({type,paths})=>{const {emitTo}=await import('/node_modules/@tauri-apps/api/event.js');await emitTo('main',`tauri://drag-${type}`,{paths,position:{x:600,y:400}})},{type,paths})}
async function addInterruptedRecord(mods){const id=require('node:crypto').randomUUID(),stage=path.join(fixture,'.mod-manager-import-'+id),source=path.join(fixture,'interrupted-source.pak'),target=path.join(mods,'interrupted-source.pak');await fs.writeFile(source,'unchanged original');await fs.mkdir(stage);await fs.writeFile(path.join(stage,'payload'),'partial owned copy');const code='import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute("INSERT INTO file_imports(id,source,target,stage,status,detail) VALUES(?,?,?,?,?,?)",sys.argv[2:]); c.commit()';const proc=spawn(process.env.QA_PYTHON||'python',['-c',code,path.join(data,'manager.sqlite3'),id,source,target,stage,'copying','Legacy interrupted fixture'],{windowsHide:true});await new Promise((r,j)=>proc.on('exit',c=>c===0?r():j(Error('journal fixture failed'))));return {id,stage,source,target};}

const http=require('node:http');let held=true,pending=[];
function forward(req,res){const upstream=http.request({hostname:'127.0.0.1',port:1421,path:req.url,method:req.method,headers:{...req.headers,host:'127.0.0.1:1421'}},r=>{res.writeHead(r.statusCode,r.headers);r.pipe(res)});upstream.on('error',e=>{res.statusCode=502;res.end(String(e))});req.pipe(upstream)}
const proxy=http.createServer((req,res)=>{if(held&&req.url.startsWith('/src/main.tsx'))pending.push([req,res]);else forward(req,res)});
async function launchGated(scheme){
 held=true;pending=[];
 const proc=spawn('powershell.exe',['-NoProfile','-Command',`$env:MM_TEST_DATA_DIR=${quote(data)};$p=Start-Process -FilePath ${quote(path.join(root,require('./dev-paths.cjs').debugExeRelative))} -WindowStyle Hidden -PassThru;Write-Output $p.Id`],{windowsHide:true});let out='';proc.stdout.on('data',b=>out+=b);await new Promise((r,j)=>proc.on('exit',c=>c===0?r():j(Error('launch failed'))));pid=Number(out.trim());
 browser=null;for(let i=0;i<40;i++){try{browser=await chromium.connectOverCDP('http://127.0.0.1:9222');break}catch{await delay(150)}}assert.ok(browser);page=browser.contexts()[0].pages()[0];
 await eventually(async()=>pending.length>0,'main module held');await page.waitForFunction(()=>window.__MM_THEME__&&document.documentElement.dataset.theme);
 const visible=()=>page.evaluate(async()=>{const{getCurrentWindow}=await import('/node_modules/@tauri-apps/api/window.js');return getCurrentWindow().isVisible()});
 assert.equal(await visible(),false,'window stays hidden before React starts');
 const bootstrap=await page.evaluate(()=>({preference:window.__MM_THEME__,theme:document.documentElement.dataset.theme,background:document.documentElement.style.backgroundColor}));
 await page.emulateMedia({colorScheme:scheme});await page.evaluate(()=>{window.__themeTrace=[document.documentElement.dataset.theme];new MutationObserver(()=>window.__themeTrace.push(document.documentElement.dataset.theme)).observe(document.documentElement,{attributes:true,attributeFilter:['data-theme']})});
 held=false;for(const pair of pending)forward(...pair);pending=[];await page.locator('.brand').waitFor();await eventually(visible,'native window shown');
 return bootstrap;
}
(async()=>{
 await fs.mkdir(path.join(root,'_local/qa'),{recursive:true});fixture=await fs.mkdtemp(path.join(root,'_local/qa/startup-theme-'));data=path.join(fixture,'data');await new Promise(r=>proxy.listen(1420,'127.0.0.1',r));
 await launchGated('light');await ipc('settings',{theme:'dark'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');await stop();
 let boot=await launchGated('light');assert.equal(boot.preference,'dark');assert.equal(boot.theme,'dark');assert.equal(boot.background,'rgb(23, 29, 40)');assert.equal(await page.evaluate(()=>document.documentElement.dataset.theme),'dark');assert.ok((await page.evaluate(()=>window.__themeTrace)).every(t=>t==='dark'));checks.push('Saved dark: injected before React/CSS; native window hidden until ready; no light theme in startup trace, even with light system');
 await ipc('settings',{theme:'light'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='light');await stop();
 boot=await launchGated('dark');assert.equal(boot.preference,'light');assert.equal(boot.theme,'light');assert.ok((await page.evaluate(()=>window.__themeTrace)).every(t=>t==='light'));checks.push('Saved light: first visible appearance stays light with dark system');
 await ipc('settings',{theme:'system'});await stop();await launchGated('dark');await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');await page.emulateMedia({colorScheme:'light'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='light');assert.equal(await page.evaluate(()=>document.documentElement.style.backgroundColor),'rgb(252, 253, 255)');await page.emulateMedia({colorScheme:'dark'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');assert.equal((await ipc('snapshot')).settings.theme,'system');checks.push('System preference resolves before show; live changes update theme and background without overwriting saved system preference');
 await page.route('**/node_modules/.vite/deps/chunk-*.js*',async route=>{
  const response=await route.fetch(),source=await response.text();
  const patched=source.replace('return window.__TAURI_INTERNALS__.invoke(cmd, args, options);',"if(cmd==='request'&&args?.request?.action==='settings'){if(window.__qaFailTheme){window.__qaFailTheme=false;throw new Error('QA settings write failed')}if(window.__qaHoldTheme)await new Promise(r=>{window.__qaReleaseTheme=r})}const result=await window.__TAURI_INTERNALS__.invoke(cmd, args, options);if(cmd==='request'&&args?.request?.action==='settings')window.__qaWrites=(window.__qaWrites||0)+1;return result;");
  await route.fulfill({response,body:patched});
 });
 await page.reload();await page.locator('.brand').waitFor();await ipc('settings',{theme:'dark'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');
 await page.getByRole('button',{name:'设置',exact:true}).click();
 await page.evaluate(()=>{window.__qaHoldTheme=true;window.__qaReleaseTheme=null;window.__qaWrites=0});
 await page.getByRole('button',{name:'浅色',exact:true}).click();
 try{await page.waitForFunction(()=>typeof window.__qaReleaseTheme==='function');await page.waitForFunction(()=>document.documentElement.dataset.theme==='light',{},{timeout:1200})}
 finally{await page.evaluate(()=>{window.__qaHoldTheme=false;window.__qaReleaseTheme?.()})}
 checks.push('Appearance updates before the deliberately held settings write is released');
 await page.waitForFunction(()=>window.__qaWrites===1);
 await eventually(async()=>(await ipc('snapshot')).settings.theme==='light','light saved');
 await page.evaluate(()=>{window.__qaHoldTheme=true;window.__qaReleaseTheme=null;window.__qaWrites=0});
 await page.getByRole('button',{name:'深色',exact:true}).click();await page.waitForFunction(()=>typeof window.__qaReleaseTheme==='function');
 await page.getByRole('button',{name:'跟随系统',exact:true}).click();await page.getByRole('button',{name:'浅色',exact:true}).click();
 assert.equal(await page.getByRole('button',{name:'浅色',exact:true}).getAttribute('aria-pressed'),'true');
 await page.evaluate(async()=>{const{emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('library-changed')});await delay(300);
 assert.equal(await page.evaluate(()=>document.documentElement.dataset.theme),'light','background snapshot cannot undo a pending preference');
 await page.evaluate(()=>{window.__qaHoldTheme=false;window.__qaReleaseTheme()});
 await page.waitForFunction(()=>window.__qaWrites===2);
 await eventually(async()=>(await ipc('snapshot')).settings.theme==='light','last rapid choice saved');
 checks.push('Rapid switches stay enabled, last choice wins, and a background library snapshot cannot revert it');
 await page.evaluate(()=>{window.__qaFailTheme=true});await page.getByRole('button',{name:'深色',exact:true}).click();
 await page.getByRole('alert').waitFor();await page.waitForFunction(()=>document.documentElement.dataset.theme==='light');
 assert.equal((await ipc('snapshot')).settings.theme,'light');checks.push('Failed save reports an error and restores the last persisted appearance');
 await fs.writeFile(path.join(root,'_local/output/startup-theme-qa.json'),JSON.stringify({passed:true,checks,fixture},null,2));console.log(JSON.stringify({passed:true,checks},null,2));await stop();proxy.close();
})().catch(async e=>{console.error(e);await stop().catch(()=>{});proxy.close();process.exitCode=1});
