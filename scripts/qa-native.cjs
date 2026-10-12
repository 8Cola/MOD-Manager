// Acceptance checks against the compiled Tauri WebView and real Rust IPC. No mocked data.
const fs = require('node:fs/promises');
const path = require('node:path');
const { spawn } = require('node:child_process');
const assert = require('node:assert/strict');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || 'playwright');
const root = path.resolve(__dirname, '..');
const output = path.join(root, 'output', 'playwright');
const checks = [];
const quoted = value => "'"+value.replaceAll("'","''")+"'";
let processHandle, browser, page, fixture, appData;
const pause = ms => new Promise(r => setTimeout(r, ms));
async function start() {
  const executable=path.join(root,'src-tauri','target','debug','mod-manager.exe');
  const command=`$env:MM_TEST_DATA_DIR=${quoted(appData)}; $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9222'; $p=Start-Process -FilePath ${quoted(executable)} -WindowStyle Hidden -PassThru; Write-Output $p.Id`;
  const launcher=spawn('powershell.exe',['-NoProfile','-Command',command],{cwd:root,windowsHide:true});
  let text=''; launcher.stdout.on('data',chunk=>{text+=chunk.toString()});
  await new Promise((resolve,reject)=>{launcher.on('exit',code=>code===0?resolve():reject(Error('Native launcher failed')));launcher.on('error',reject)});
  const pid=Number(text.trim());if(!Number.isInteger(pid))throw Error('Native PID missing');
  processHandle={kill:()=>process.kill(pid)};
  for(let i=0;i<40;i++) {
    try { browser=await chromium.connectOverCDP('http://127.0.0.1:9222'); break; } catch {await pause(500);}
  }
  if(!browser) throw Error('Native WebView CDP did not start');
  for(let i=0;i<30;i++) {
    page=browser.contexts()[0].pages().find(p=>p.url().includes('1420')) || browser.contexts()[0].pages()[0];
    if(page) break;
    await pause(300);
  }
  if(!page) throw Error('Tauri UI page missing');
  await page.locator('.brand').waitFor();
  await page.emulateMedia({reducedMotion:'reduce'});
}
async function stop() {
  if(browser) {await browser.close().catch(()=>{});browser=null;}
  if(processHandle) { try {processHandle.kill();} catch {} await pause(800); }
}
async function req(action,args={}) {
  return page.evaluate(async ({action,args})=>{
    const api=await import('/src/api.ts');
    return api.request(action,args);
  },{action,args});
}
async function until(predicate,timeout=18000) {
  const began=Date.now();
  while(Date.now()-began<timeout){const s=await req('snapshot');if(await predicate(s))return s;await pause(350);}
  throw Error('Timed out waiting for real backend state');
}
async function check(name,fn) {await fn();checks.push({name,result:'PASS'});console.log('PASS',name);}
async function shot(name) {await page.screenshot({path:path.join(output,name+'.png'),animations:'disabled'});}
(async()=>{
  await fs.mkdir(output,{recursive:true});
  await fs.mkdir(path.join(root,'.tools','qa'),{recursive:true});
  fixture=await fs.mkdtemp(path.join(root,'.tools','qa','native-'));
  appData=path.join(fixture,'app-data');
  const mods=path.join(fixture,'mods'); const folderMods=path.join(fixture,'folder-mods');
  await fs.mkdir(mods);await fs.mkdir(folderMods);
  await start();
  let gameId,dressId,singleId,manualId;
  await check('Native UI creates game with independent real folder',async()=>{
    await page.getByRole('button',{name:'添加第一个游戏',exact:true}).click();
    const dialog=page.getByRole('dialog',{name:'添加游戏'});
    await dialog.getByLabel('游戏名称',{exact:true}).fill('Stellar Blade · 测试目录');
    await dialog.getByLabel('目录 1 路径',{exact:true}).fill(mods);
    await dialog.getByRole('button',{name:'保存游戏',exact:true}).click();
    await dialog.waitFor({state:'hidden'});
    const s=await req('snapshot');gameId=s.games[0].id;
    await req('saveGame',{game:{...s.games[0],imagePath:path.join(root,'src-tauri','icons','icon.png')}});
    assert.equal(s.games.length,1);assert.equal(s.mods.length,0);
  });
  await check('Rounded dialog contains scroll; game image preview is square; no title strip',async()=>{
    await page.getByRole('button',{name:'编辑游戏',exact:true}).click();
    const d=page.getByRole('dialog',{name:'编辑游戏'});
    await page.waitForFunction(()=>{const i=document.querySelector('.picture-preview img');return i&&i.complete&&i.naturalWidth>0});
    const box=await d.locator('.picture-preview').boundingBox();assert.equal(box.width,box.height);
    assert.equal(await page.getByPlaceholder('自动获取本地图标').count(),0);
    assert.equal(await d.evaluate(e=>getComputedStyle(e).overflow),'hidden');
    await page.setViewportSize({width:860,height:600});
    assert.ok(await d.locator('.modal-body').evaluate(e=>e.scrollHeight>e.clientHeight));
    assert.ok(await d.locator('.modal-body').evaluate(e=>{const b=e.getBoundingClientRect(),m=e.closest('.modal').getBoundingClientRect();return b.left>m.left&&b.right<m.right&&b.top>m.top&&b.bottom<m.bottom}));
    await shot('fixed-dialog-narrow');
    await d.locator('.modal-body').evaluate(e=>e.scrollTop=e.scrollHeight);
    const action=await d.getByRole('button',{name:'保存游戏',exact:true}).boundingBox();assert.ok(action.y+action.height<=600);
    await page.keyboard.press('Escape');await page.setViewportSize({width:1240,height:820});
    assert.equal(await page.locator('.window-bar').evaluate(e=>getComputedStyle(e).backgroundColor),'rgba(0, 0, 0, 0)');
    assert.equal(await page.locator('.app-shell').evaluate(e=>getComputedStyle(e).paddingTop),'0px');
    assert.equal(await page.locator('.window-bar').innerText(),'');
    await shot('fixed-integrated-window');
  });
  await check('Real notify discovers copied single/multi/unknown/long filenames',async()=>{
    for(const ext of ['pak','utoc','ucas']) await fs.writeFile(path.join(mods,'EVE_BlackDress_P.'+ext),'dress-'+ext);
    await fs.writeFile(path.join(mods,'Camera.pak'),'camera-original');
    await fs.writeFile(path.join(mods,'Lighting.pak'),'lighting-original');
    await fs.writeFile(path.join(mods,'readme.txt'),'not confirmed mod');
    await fs.writeFile(path.join(mods,'Very_Long_Mod_'+('x'.repeat(100))+'.pak'),'long');
    const s=await until(s=>s.mods.filter(m=>m.gameId===gameId).length===5);
    const dress=s.mods.find(m=>m.members.length===3);assert.ok(dress);dressId=dress.id;
    singleId=s.mods.find(m=>m.members[0].originalPath.endsWith('Camera.pak')).id;
    assert.equal(s.mods.find(m=>m.members[0].originalPath.endsWith('readme.txt')).managed,false);
  });
  await check('UI saves long note; search matches notes',async()=>{
    await page.locator('.mod-row').filter({hasText:'EVE_BlackDress_P'}).getByRole('button',{name:/查看 .* 详情/}).click();
    const d=page.getByRole('dialog',{name:'MOD 详情'});
    // Names now follow real files; metadata editing preserves the original paths.
    await d.getByLabel('描述与备注',{exact:true}).fill('替换默认服装，适合拍照。与柔和灯光一起使用。\n'+('中文长备注。'.repeat(80)));
    await d.getByRole('button',{name:'保存信息',exact:true}).click();
    await until(s=>s.mods.find(m=>m.id===dressId).description.includes('柔和灯光'));
    await d.getByRole('button',{name:'关闭',exact:true}).click();
    await page.getByRole('textbox',{name:'搜索 Mod'}).fill('柔和灯光');
    await page.locator('.mod-name-text').filter({hasText:'EVE_BlackDress_P'}).waitFor();
    assert.equal(await page.locator('.mod-row').count(),1);
    await page.getByRole('textbox',{name:'搜索 Mod'}).fill('');
  });
  await check('Single and entire group toggles preserve bytes',async()=>{
    for(const id of [singleId,dressId]){
      assert.equal((await req('toggle',{ids:[id],enabled:false})).results[0].success,true);
      assert.equal((await req('snapshot')).mods.find(m=>m.id===id).state,'disabled');
      assert.equal((await req('toggle',{ids:[id],enabled:true})).results[0].success,true);
    }
    assert.equal(await fs.readFile(path.join(mods,'Camera.pak'),'utf8'),'camera-original');
    assert.equal(await fs.readFile(path.join(mods,'EVE_BlackDress_P.utoc'),'utf8'),'dress-utoc');
  });
  await check('Manual group/add/remove/ungroup has exclusive membership',async()=>{
    let s=await req('snapshot');const lighting=s.mods.find(m=>m.members[0].originalPath.endsWith('Lighting.pak'));
    await page.getByRole('button',{name:'批量管理',exact:true}).click();
    await page.getByLabel('选择 Camera.pak',{exact:true}).check();
    await page.getByLabel('选择 Lighting.pak',{exact:true}).check();
    await page.getByRole('button',{name:'创建分组',exact:true}).click();
    const dialog=page.getByRole('dialog',{name:'创建逻辑 Mod'});

    await dialog.getByLabel('备注',{exact:true}).fill('手动分组');
    await dialog.getByRole('button',{name:'创建分组',exact:true}).click();
    await dialog.waitFor({state:'hidden'});
    s=await req('snapshot');
    let g=s.mods.find(m=>m.description==='手动分组');manualId=g.id;assert.equal(g.members.length,2);
    const kept=g.members[0].id;
    s=await req('setMembers',{id:g.id,memberIds:[kept]});assert.equal(s.mods.find(m=>m.id===g.id).members.length,1);
    const other=s.mods.find(m=>m.members.some(f=>f.originalPath.endsWith('Lighting.pak')));
    s=await req('group',{ids:[g.id,other.id],name:'镜头与灯光',description:'手动分组'});
    g=s.mods.find(m=>m.name==='镜头与灯光');
    await req('ungroup',{id:g.id});
    s=await req('snapshot');assert.equal(new Set(s.mods.flatMap(m=>m.members.map(f=>f.id))).size,s.mods.flatMap(m=>m.members).length);
  });
  await check('Folder MOD uses outside-root move and restores original location',async()=>{
    await fs.mkdir(path.join(folderMods,'Costume'));await fs.writeFile(path.join(folderMods,'Costume','texture.bin'),'folder-bytes');
    let s=await req('saveGame',{game:{name:'文件夹型 Mod',modDir:folderMods,strategy:'move',imagePath:null,exePath:null}});
    const g=s.games.find(g=>g.modDir.toLowerCase().replaceAll('\\','/').endsWith('/folder-mods'));
    const m=s.mods.find(m=>m.gameId===g.id);assert.equal(m.members[0].kind,'directory');
    await req('editMod',{id:m.id,managed:true,name:'整套服装',description:'包含完整文件夹'});
    assert.equal((await req('toggle',{ids:[m.id],enabled:false})).results[0].success,true);
    await assert.rejects(fs.stat(path.join(folderMods,'Costume')));
    assert.equal((await req('toggle',{ids:[m.id],enabled:true})).results[0].success,true);
    assert.equal(await fs.readFile(path.join(folderMods,'Costume','texture.bin'),'utf8'),'folder-bytes');
  });
  await check('Batch result distinguishes unknown failure; confirmed files restored',async()=>{
    const s=await req('snapshot');const ids=s.mods.filter(m=>m.gameId===gameId).map(m=>m.id);
    if(await page.getByRole('button',{name:'完成选择',exact:true}).count()) await page.getByRole('button',{name:'完成选择',exact:true}).click();
    await page.getByRole('button',{name:'批量管理',exact:true}).click();
    await page.getByLabel('选择当前结果',{exact:true}).check();
    await shot('native-selection');
    await page.locator('.batch-bar').getByRole('button',{name:'关闭',exact:true}).click();
    await until(s=>s.mods.filter(m=>m.gameId===gameId&&m.managed).every(m=>m.state==='disabled'));
    await page.locator('.progress-toast').waitFor({state:'hidden'});
    assert.ok((await page.locator('.error-toast').innerText()).includes('纳入管理'));
    await shot('native-batch-partial-error');
    await page.locator('.batch-bar').getByRole('button',{name:'开启',exact:true}).click();
    await until(s=>s.mods.filter(m=>m.gameId===gameId&&m.managed).every(m=>m.state==='enabled'));
    await page.locator('.progress-toast').waitFor({state:'hidden'});
    await page.getByRole('button',{name:'完成选择',exact:true}).click();
    await page.getByRole('button',{name:'关闭错误',exact:true}).click();
  });
  await check('Deleted and recreated empty watch root registers and discovers new file',async()=>{
    const empty=path.join(fixture,'empty-watch-root');await fs.mkdir(empty);
    const s=await req('saveGame',{game:{name:'空目录监控',modDir:empty,strategy:'suffix',exePath:null,imagePath:null}});
    const g=s.games.find(g=>g.name==='空目录监控');
    await pause(3500);await fs.rmdir(empty);await fs.mkdir(empty);await fs.writeFile(path.join(empty,'After_Recreate.pak'),'new');
    await until(s=>s.mods.some(m=>m.gameId===g.id&&m.members[0].originalPath.endsWith('After_Recreate.pak')));
  });
  await check('External rename/deletion/addition retains old notes and syncs list',async()=>{
    await fs.rename(path.join(mods,'Camera.pak'),path.join(mods,'Camera_Renamed.pak'));
    await until(s=>s.mods.some(m=>m.members.some(f=>f.originalPath.endsWith('Camera_Renamed.pak'))));
    await fs.unlink(path.join(mods,'Very_Long_Mod_'+('x'.repeat(100))+'.pak'));
    await until(s=>s.mods.some(m=>m.state==='missing'));
    assert.equal((await req('snapshot')).mods.find(m=>m.id===dressId).description.includes('柔和灯光'),true);
  });
  await check('Persisted custom game image is rendered from app data',async()=>{
    const s=await req('snapshot');assert.ok(s.games[0].imagePath.startsWith(appData));
    await page.reload();await page.waitForSelector('.game-image img');
    await page.waitForFunction(()=>[...document.querySelectorAll('.game-image img')].every(i=>i.complete&&i.naturalWidth>0));
  });
  await check('MOD cover renders, backs up, restores and clears independently',async()=>{
    await req('editMod',{id:dressId,imagePath:path.join(root,'src-tauri','icons','icon.png')});
    await page.waitForFunction(()=>[...document.querySelectorAll('.mod-symbol img')].some(i=>i.complete&&i.naturalWidth>0));
    await page.getByRole('button',{name:/查看 EVE_BlackDress_P.* 详情/}).click();
    const d=page.getByRole('dialog',{name:'MOD 详情'});
    await d.getByRole('button',{name:'更换封面',exact:true}).waitFor();
    await shot('native-detail-cover-light');
    await d.getByRole('button',{name:'关闭',exact:true}).click();
    const file=path.join(fixture,'cover-backup.json');await req('export',{path:file});
    await req('editMod',{id:dressId,imagePath:null});
    assert.equal((await req('snapshot')).mods.find(m=>m.id===dressId).imagePath,null);
    await req('import',{path:file});
    assert.ok((await req('snapshot')).mods.find(m=>m.id===dressId).imagePath);
    await page.reload();await page.waitForSelector('.mod-symbol img');
    await page.waitForFunction(()=>[...document.querySelectorAll('.mod-symbol img')].every(i=>i.complete&&i.naturalWidth>0));
  });
  await check('Live system/manual light/dark appearance and keyboard modal close',async()=>{
    await page.getByRole('button',{name:'设置',exact:true}).click();
    await page.getByRole('button',{name:'深色',exact:true}).click();
    await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');
    await shot('native-settings-dark');await page.keyboard.press('Escape');
    await shot('native-library-dark');
    await req('settings',{theme:'system'});await page.emulateMedia({colorScheme:'light'});await page.reload();
    await page.waitForFunction(()=>document.documentElement.dataset.theme==='light');
    await shot('native-library-light');
    await page.emulateMedia({colorScheme:'dark'});
    await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');
    await page.setViewportSize({width:860,height:600});await shot('native-narrow-dark');
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth));
  });
  await check('Versioned backup restores metadata/images without changing mod bytes',async()=>{
    const backup=path.join(fixture,'backup.json');await req('export',{path:backup});
    const content=JSON.parse(await fs.readFile(backup,'utf8'));assert.ok(content.version);
    await req('editMod',{id:dressId,name:'临时改名',description:'临时',managed:true});
    await req('import',{path:backup});
    assert.equal((await req('snapshot')).mods.find(m=>m.id===dressId).description.includes('柔和灯光'),true);
    assert.equal(await fs.readFile(path.join(mods,'EVE_BlackDress_P.pak'),'utf8'),'dress-pak');
  });
  await check('Replaced file needs explicit UI consent before reassociation',async()=>{
    await fs.unlink(path.join(mods,'EVE_BlackDress_P.pak'));await fs.writeFile(path.join(mods,'EVE_BlackDress_P.pak'),'user-verified replacement');
    await until(s=>s.mods.find(m=>m.id===dressId).identityChanged===true);
    await page.getByRole('button',{name:/查看 EVE_BlackDress_P.* 详情/}).click();
    const d=page.getByRole('dialog',{name:'MOD 详情'});
    await d.getByRole('button',{name:'核对当前文件',exact:true}).click();
    const confirm=d.getByRole('button',{name:'确认关联',exact:true});assert.equal(await confirm.isDisabled(),true);
    await shot('native-replacement-confirmation');
    await d.getByLabel(/我已核对成员文件/).check();await confirm.click();
    await until(s=>s.mods.find(m=>m.id===dressId).state==='enabled'&&!s.mods.find(m=>m.id===dressId).identityChanged);
    assert.equal(await fs.readFile(path.join(mods,'EVE_BlackDress_P.pak'),'utf8'),'user-verified replacement');
    await d.getByRole('button',{name:'关闭',exact:true}).click();
  });
  await check('Actual process restart restores SQLite metadata and settings',async()=>{
    await req('settings',{theme:'dark'});await stop();await start();
    const s=await req('snapshot');assert.equal(s.mods.find(m=>m.id===dressId).description.includes('柔和灯光'),true);assert.equal(s.settings.theme,'dark');
    await shot('native-after-restart');
  });
  await check('Window buttons maximize/restore; transitions honor reduced motion',async()=>{
    const before=(await page.getByRole('button',{name:'还原窗口',exact:true}).count())>0;
    await page.getByRole('button',{name:before?'还原窗口':'最大化',exact:true}).click();
    await pause(500);
    await page.getByRole('button',{name:before?'最大化':'还原窗口',exact:true}).waitFor();
    await page.getByRole('button',{name:before?'最大化':'还原窗口',exact:true}).click();
    await pause(500);
    await page.emulateMedia({reducedMotion:'no-preference'});
    await page.getByRole('button',{name:'设置',exact:true}).click();
    assert.equal(await page.locator('.page-view').evaluate(e=>getComputedStyle(e).animationName),'page-enter');
    await page.emulateMedia({reducedMotion:'reduce'});
    assert.ok(parseFloat(await page.locator('.page-view').evaluate(e=>getComputedStyle(e).animationDuration))<0.001);
    await page.keyboard.press('Escape');
    await page.setViewportSize({width:1240,height:820});
    await req('settings',{theme:'light'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='light');
    await shot('redesign-library-light');
    await page.getByRole('button',{name:/查看 EVE_BlackDress_P.* 详情/}).click();
    await shot('redesign-detail-light');await page.keyboard.press('Escape');
    await page.getByRole('button',{name:'编辑游戏',exact:true}).click();
    await shot('redesign-game-editor');await page.keyboard.press('Escape');
    await page.getByRole('button',{name:'设置',exact:true}).click();await shot('redesign-settings-light');
    await page.getByRole('button',{name:'深色',exact:true}).click();await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');
    await shot('redesign-settings-dark');await page.keyboard.press('Escape');
    await shot('redesign-library-dark');
    await page.getByRole('button',{name:/查看 EVE_BlackDress_P.* 详情/}).click();await shot('redesign-detail-dark');await page.keyboard.press('Escape');
  });
  await stop();
  await fs.writeFile(path.join(root,'output','native-qa.json'),JSON.stringify({checks,fixture,appData},null,2));
  console.log(`${checks.length} native acceptance checks passed. Fixtures retained at ${fixture}`);
})().catch(async error=>{
  console.error(error);checks.push({name:'Interrupted',result:'FAIL',error:String(error)});
  await fs.writeFile(path.join(root,'output','native-qa.json'),JSON.stringify({checks,fixture,appData},null,2)).catch(()=>{});
  await stop();process.exitCode=1;
});
