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
(async()=>{
 await fs.mkdir(path.join(root,'_local/qa'),{recursive:true}); await fs.mkdir(path.join(root,'_local/output/playwright'),{recursive:true});
 fixture=await fs.mkdtemp(path.join(root,'_local/qa/clipboard-'));data=path.join(fixture,'data');const mods=path.join(fixture,'mods');await fs.mkdir(mods);await fs.writeFile(path.join(mods,'first.pak'),'original');
 const source=path.join(root,'src-tauri/tests/fixtures/crop512.png');
 const helper=path.join(fixture,'clipboard.ps1');
 await fs.writeFile(helper,`
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$backup=New-Object System.Windows.Forms.DataObject
$original=[System.Windows.Forms.Clipboard]::GetDataObject()
if($null -ne $original) { foreach($format in $original.GetFormats($false)) { $value=$original.GetData($format,$false); if($value -is [System.Drawing.Image]){$value=$value.Clone()}; $backup.SetData($format,$false,$value) } }
$image=[System.Drawing.Image]::FromFile(${quote(source)})
try {
 [System.Windows.Forms.Clipboard]::SetImage($image)
 [System.IO.File]::WriteAllText(${quote(path.join(fixture,'ready'))},'ready')
 $until=[DateTime]::UtcNow.AddMinutes(3)
 while(-not(Test-Path -LiteralPath ${quote(path.join(fixture,'done'))}) -and [DateTime]::UtcNow -lt $until) {
  if(Test-Path -LiteralPath ${quote(path.join(fixture,'text'))}) { [System.Windows.Forms.Clipboard]::SetText('clipboard text fixture'); Remove-Item -LiteralPath ${quote(path.join(fixture,'text'))}; [System.IO.File]::WriteAllText(${quote(path.join(fixture,'text-ready'))},'ready') }
  Start-Sleep -Milliseconds 100
 }
} finally { [System.Windows.Forms.Clipboard]::SetDataObject($backup,$true); $image.Dispose() }
`);
 const clip=spawn('powershell.exe',['-NoProfile','-NonInteractive','-Sta','-File',helper],{windowsHide:true});let clipError='';clip.stderr.on('data',b=>clipError+=b);
 const clipExit=new Promise(resolve=>clip.on('exit',resolve));
 try {
 await eventually(async()=>fs.stat(path.join(fixture,'ready')).then(()=>true).catch(()=>false),'clipboard fixture ready');
 await start();await ipc('saveGame',{game:{name:'剪贴板验收',strategy:'suffix',folders:[{name:'MOD',path:mods}]}});await page.locator('.mod-row').waitFor();
 const clipboard=await ipc('clipboardImage');assert.ok(clipboard.url.startsWith('data:image/png;base64,'));checks.push('Real Windows clipboard bitmap encoded by native IPC');
 await page.getByLabel('编辑游戏与目录').click();const gamePicker=page.getByRole('button',{name:'游戏图片来源',exact:true});await gamePicker.click();await page.getByRole('menu',{name:'图片来源'}).waitFor();assert.equal(await page.getByRole('dialog',{name:'选好画面，再保存'}).count(),0);
 await page.getByRole('menuitem',{name:'粘贴图片'}).click();await page.getByRole('dialog',{name:'选好画面，再保存'}).waitFor();await page.getByRole('button',{name:'取消',exact:true}).last().click();assert.equal((await ipc('snapshot')).games[0].imagePath,null);checks.push('Game square opens source menu; paste opens crop; cancel preserves image');
 await gamePicker.focus();await gamePicker.press('Control+v');await page.getByRole('dialog',{name:'选好画面，再保存'}).waitFor();await page.getByRole('button',{name:'确认裁剪'}).click();await page.getByRole('dialog',{name:'选好画面，再保存'}).waitFor({state:'hidden'});await page.getByRole('button',{name:'保存游戏'}).click();await page.getByRole('dialog').waitFor({state:'hidden'});const game=(await ipc('snapshot')).games[0];assert.equal((await decodeImage(game.imagePath)).width,512);checks.push('Focused game square Ctrl+V crops and saves actual512PNG');
 await page.locator('.mod-open').click();const cover=page.getByRole('button',{name:'查看封面原图',exact:true});await page.getByRole('button',{name:'粘贴图片',exact:true}).click();await page.getByRole('dialog',{name:'选好画面，再保存'}).waitFor();await page.getByRole('button',{name:'确认裁剪'}).click();await page.getByRole('dialog',{name:'选好画面，再保存'}).waitFor({state:'hidden'});const before=(await ipc('snapshot')).mods[0].imagePath;assert.equal((await decodeImage(before)).width,512);assert.ok((await ipc('snapshot')).mods[0].originalImagePath);checks.push('Permanent MOD paste control saves cropped cover and original');
 await cover.focus();await cover.press('Control+v');await page.getByRole('dialog',{name:'选好画面，再保存'}).waitFor();await page.getByRole('button',{name:'取消',exact:true}).last().click();assert.equal((await ipc('snapshot')).mods[0].imagePath,before);checks.push('MOD square Ctrl+V and cropcancel preserve previous cover');
 await fs.writeFile(path.join(fixture,'text'),'text');await eventually(async()=>fs.stat(path.join(fixture,'text-ready')).then(()=>true).catch(()=>false),'clipboardtext');
 const note=page.getByLabel('描述与备注',{exact:true});await note.focus();await note.press('Control+v');await eventually(async()=>await note.inputValue()==='clipboard text fixture','normal text paste');assert.equal(await page.getByRole('dialog',{name:'选好画面，再保存'}).count(),0);checks.push('Notes Ctrl+V pastes actual text without invoking image crop');
 await page.getByRole('button',{name:'粘贴图片',exact:true}).click();await page.getByRole('alert').filter({hasText:'剪贴板中没有图片'}).waitFor();assert.equal(await page.getByRole('dialog',{name:'MOD 详情'}).count(),1);checks.push('Permanent paste button reports clipboard without image');
 await interceptPicker(source);await page.getByRole('button',{name:'更换封面',exact:true}).click();await page.getByRole('dialog',{name:'选好画面，再保存'}).waitFor();await page.getByRole('button',{name:'取消',exact:true}).last().click();checks.push('Permanent replace control preserves filepicker alternative and same crop workflow');
 await page.waitForFunction(()=>!document.querySelector('[aria-label="选好画面，再保存"]'));await delay(400);await page.screenshot({path:path.join(root,'_local/output/playwright/clipboard-menu-light.png')});
 const ui=await page.locator('body').innerText();assert.ok(!/\bMod\b/.test(ui.replaceAll('MOD Manager','').replaceAll('Mod-Manager','')));assert.equal(await fs.readFile(path.join(mods,'first.pak'),'utf8'),'original');checks.push('Visible labels MOD; brand MOD Manager; realfilename/content unchanged');
 await fs.writeFile(path.join(root,'_local/output/clipboard-native-qa.json'),JSON.stringify({passed:true,checks,fixture,notes:'Real system clipboard image/text, native IPC, WebView keyboard paste. Only filepicker result routed to fixture. Clipboard restored in helper finally.'},null,2));console.log(JSON.stringify({passed:true,checks},null,2));
 } finally { await stop();await fs.writeFile(path.join(fixture,'done'),'done');const code=await clipExit;assert.equal(code,0,clipError); }
})().catch(async error=>{console.error(error);if(page)await page.screenshot({path:path.join(root,'_local/output/playwright/clipboard-failure.png')}).catch(()=>{});await stop().catch(()=>{});process.exitCode=1});
