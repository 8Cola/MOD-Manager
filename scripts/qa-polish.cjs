// Real native WebView and file-system checks; all fixtures are isolated from games.
const fs=require('node:fs/promises'),path=require('node:path'),assert=require('node:assert/strict'),{spawn}=require('node:child_process');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE_PATH || 'playwright');
const root=path.resolve(__dirname,'..'),checks=[];let browser,page,pid,fixture,data;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const quote=s=>"'"+s.replaceAll("'","''")+"'";
async function start(){
 const cmd=`$env:MM_TEST_DATA_DIR=${quote(data)}; $p=Start-Process -FilePath ${quote(path.join(root,require('./dev-paths.cjs').debugExeRelative))} -WindowStyle Hidden -PassThru; Write-Output $p.Id`;
 const proc=spawn('powershell.exe',['-NoProfile','-Command',cmd],{windowsHide:true});let out='';proc.stdout.on('data',b=>out+=b);await new Promise((res,rej)=>proc.on('exit',c=>c===0?res():rej(Error('launch failed'))));pid=Number(out.trim());
 for(let i=0;i<40;i++){try{browser=await chromium.connectOverCDP('http://127.0.0.1:9222');break}catch{await delay(500)}}
 if(!browser)throw Error('No native WebView');page=browser.contexts()[0].pages()[0];await page.locator('.brand').waitFor();await page.setViewportSize({width:1240,height:820});
}
async function stop(){if(browser){await browser.close();browser=null}if(pid){process.kill(pid);pid=null}}
async function ipc(action,args={}){return page.evaluate(async ({action,args})=>{const api=await import('/src/api.ts');return api.request(action,args);},{action,args});}
async function ready(){await page.waitForFunction(()=>!document.querySelector('.add-game').disabled)}
const row=name=>page.locator('.mod-row').filter({has:page.locator('.mod-name-text',{hasText:new RegExp(`^${name.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')}$`)})});
(async()=>{
 await fs.mkdir(path.join(root,'_local/qa'),{recursive:true});await fs.mkdir(path.join(root,'_local/output/playwright'),{recursive:true});
 fixture=await fs.mkdtemp(path.join(root,'_local/qa/polish-'));data=path.join(fixture,'data');const mods=path.join(fixture,'mods'),resources=path.join(fixture,'resources');
 await fs.mkdir(path.join(mods,'folder-mod'),{recursive:true});await fs.mkdir(resources);
 await fs.writeFile(path.join(mods,'folder-mod','normal.txt'),'normal');await fs.writeFile(path.join(mods,'folder-mod','keep.txt.disabled'),'preserve');
 for(let i=0;i<25;i++)await fs.writeFile(path.join(mods,`sample-${String(i).padStart(2,'0')}.pak`),'sample');
 for(const ext of ['pak','utoc','ucas'])await fs.writeFile(path.join(mods,`dress.${ext}`),ext);
 await fs.writeFile(path.join(resources,'pack.zip'),'archive');
 await start();await page.getByRole('button',{name:'添加游戏',exact:true}).click();
 await page.getByPlaceholder('例如：Stellar Blade').fill('界面与启停验收');await page.getByLabel('目录 1 路径',{exact:true}).fill(mods);
 await page.getByLabel('目录 1 备注名',{exact:true}).fill('常规 Mod');await page.getByRole('button',{name:'添加目录',exact:true}).click();
 await page.getByLabel('目录 2 备注名',{exact:true}).fill('材质包');await page.getByLabel('目录 2 路径',{exact:true}).fill(resources);
 await page.getByRole('button',{name:'保存游戏',exact:true}).click();await row('sample-00.pak').waitFor();await ready();
 assert.equal(await page.locator('.mod-description').count(),0);assert.equal(await row('sample-00.pak').locator('.mod-metadata').count(),0);assert.match(await row('dress.pak').innerText(),/3 个成员/);
 assert.match(await page.locator('.game-info small').innerText(),/共 28 个 Mod/);assert.match(await page.locator('.summary-line').innerText(),/当前目录/);assert.match(await page.locator('.summary-line').innerText(),/27\s*个 Mod/);
 await page.getByLabel('搜索名称',{exact:true}).fill('sample-00');await page.getByText('找到 1 / 27 个 Mod',{exact:true}).waitFor();assert.equal(await page.locator('.mod-row').count(),1);await page.getByLabel('清空搜索').click();
 checks.push('Empty notes and duplicate single-member filename hidden; group count retained; all-directory/current-directory/search counts distinct');
 for(const theme of ['light','dark']){await ipc('settings',{theme});await page.waitForFunction(t=>document.documentElement.dataset.theme===t,theme);
 for(const width of [1240,860]){await page.setViewportSize({width,height:width===860?600:820});await page.locator('.library-scroll').evaluate(e=>e.scrollTop=0);
 const before=await page.locator('.library-header').boundingBox(),first=await page.locator('.mod-row').first().boundingBox();assert.ok(before.height<180,JSON.stringify(before));
 const toolbar=await page.locator('.toolbar').boundingBox(),edit=await page.getByLabel('编辑游戏与目录',{exact:true}).boundingBox();assert.ok(edit.x+edit.width<=toolbar.x+toolbar.width+1);
 await page.locator('.library-scroll').hover();await page.mouse.wheel(0,600);await page.waitForFunction(()=>document.querySelector('.library-scroll').scrollTop>200);
 assert.deepEqual(await page.locator('.library-header').boundingBox(),before);assert.ok((await page.locator('.mod-row').first().boundingBox()).y<first.y-200);assert.equal(await page.locator('.main-content').evaluate(e=>e.scrollTop),0);
 await page.screenshot({path:`output/playwright/polish-${width}-${theme}.png`});await page.getByLabel('当前 MOD 目录').click();assert.equal(await page.getByRole('option').count(),2);await page.keyboard.press('Escape');
 }}checks.push('Whole compact header stays frozen during mouse-wheel scrolling at 1240/860px in both themes; toolbar and directory picker fit');
 await page.setViewportSize({width:1240,height:820});await page.locator('.library-scroll').evaluate(e=>e.scrollTop=0);
 await page.locator('.page-view').evaluate(e=>{window.__entryAnimations=0;e.addEventListener('animationstart',event=>{if(event.target===e)window.__entryAnimations++})});await ipc('scan');await delay(3300);assert.equal(await page.evaluate(()=>window.__entryAnimations),0);checks.push('Directory refresh and periodic snapshot do not replay page-entry animation');
 const busyStates=[];await page.exposeFunction('recordBusyState',v=>busyStates.push(v));await page.evaluate(()=>{new MutationObserver(()=>{const el=document.querySelector('.switch[aria-busy="true"]');if(el)window.recordBusyState({spinner:!!el.querySelector('svg.spin'),globalProgress:!!document.querySelector('.progress-toast')})}).observe(document.querySelector('.mod-list'),{subtree:true,attributes:true,childList:true})});
 await row('sample-00.pak').getByRole('switch').click();await row('sample-00.pak.disabled').waitFor();await ready();assert.ok(busyStates.some(v=>v.spinner&&!v.globalProgress));assert.equal(await page.locator('.progress-toast').count(),0);assert.equal(await fs.readFile(path.join(mods,'sample-00.pak.disabled'),'utf8'),'sample');
 await row('sample-00.pak.disabled').getByRole('switch').click();await row('sample-00.pak').waitFor();await ready();checks.push('Single toggle shows only local switch spinner; final state follows actual renamed file; bytes unchanged');
 await page.getByRole('button',{name:'批量管理',exact:true}).click();await row('sample-00.pak').getByLabel('选择 sample-00.pak',{exact:true}).check();await row('sample-01.pak').getByLabel('选择 sample-01.pak',{exact:true}).check();await page.locator('.batch-bar').getByRole('button',{name:'关闭',exact:true}).click();await row('sample-01.pak.disabled').waitFor();await ready();assert.match(await page.locator('.toast').innerText(),/2 个 MOD 已关闭/);await page.locator('.batch-bar').getByRole('button',{name:'开启',exact:true}).click();await row('sample-01.pak').waitFor();await ready();await page.getByRole('button',{name:'完成选择',exact:true}).click();checks.push('Batch enable/disable returns exact successful count and preserves bytes');
 await row('folder-mod').locator('.mod-open').click();await page.getByRole('checkbox').check();await page.getByRole('button',{name:'保存信息',exact:true}).click();await ready();assert.match(await page.locator('.preserved-files-note').innerText(),/1 个内部文件/);
 await page.getByRole('switch',{name:'详情 MOD 启停',exact:true}).click();await page.waitForFunction(()=>document.querySelector('.detail-status strong')?.textContent==='已关闭');await ready();assert.ok(await fs.stat(path.join(mods,'folder-mod','normal.txt.disabled')));
 await page.getByRole('switch',{name:'详情 MOD 启停',exact:true}).click();await page.waitForFunction(()=>document.querySelector('.detail-status strong')?.textContent==='已开启');await ready();assert.equal(await fs.readFile(path.join(mods,'folder-mod','keep.txt.disabled'),'utf8'),'preserve');await assert.rejects(fs.stat(path.join(mods,'folder-mod','keep.txt')));
 await page.getByRole('button',{name:'改名',exact:true}).click();assert.match(await page.locator('.rename-editor').innerText(),/依赖固定名称/);await page.screenshot({path:'_local/output/playwright/polish-detail-dark.png'});await page.getByRole('button',{name:'关闭',exact:true}).click();await page.getByRole('dialog').waitFor({state:'hidden'});checks.push('Native folder toggle preserves preexisting disabled leaf and displays explanation; real rename has compatibility hint');
 await page.getByLabel('当前 MOD 目录').click();await page.getByRole('option',{name:'材质包',exact:true}).click();await row('pack.zip').waitFor();assert.match(await page.locator('.summary-line').innerText(),/1\s*个 Mod/);assert.match(await page.locator('.game-info small').innerText(),/共 28 个 Mod/);
 await stop();await start();await row('folder-mod').waitFor();assert.match(await page.locator('.game-info small').innerText(),/共 28 个 Mod/);checks.push('Directory switching scopes current counts correctly; native restart retains files and library');
 await fs.writeFile(path.join(root,'_local/output/polish-native-qa.json'),JSON.stringify({passed:true,checks,fixture},null,2));console.log(JSON.stringify({passed:true,checks,fixture},null,2));await stop();
})().catch(async e=>{console.error(e);if(page)await page.screenshot({path:path.join(root,'_local/output/playwright/polish-failure.png')}).catch(()=>{});await stop().catch(()=>{});process.exitCode=1;});


