import { ImageSourcePicker } from './components/ImageSourcePicker';
import { useCallback, useDeferredValue, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { applyTheme, startupTheme } from './theme';
import { listen } from '@tauri-apps/api/event';
import { open, save } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ClipboardPaste, ZoomIn, ZoomOut, ArrowUpRight, ArrowLeft, Check, CheckCheck, ChevronRight, CircleHelp, File, Folder, FolderOpen, Gamepad2, Layers, LoaderCircle, Monitor, Moon, Plus, RefreshCw, Search, Settings, ShieldCheck, SlidersHorizontal, Sun, Trash2, Upload, Download, X, AlertCircle, Minus, Square, Copy, Play, Box, Database, Info } from 'lucide-react';
import { desktop, imageUrl, request } from './api';
import { FolderEditor } from './components/FolderEditor';
import { DirectoryPicker } from './components/DirectoryPicker';
import { ModContextMenu } from './components/ModContextMenu';
import { ModImage, ModListRow } from './components/ModListRow';
import { DropImportOverlay, type DropTarget, type ImportProgress } from './components/DropImportOverlay';
import { ImageCropper } from './components/ImageCropper';
import type { PendingImport } from './types';
import appIconUrl from './assets/app-icon.svg';
import { version as appVersion } from '../package.json';
import { basename, displayPath, modName, memberSummary, type ViewPreferences, type Game, type Mod, type Snapshot, type Strategy, type Theme } from './types';

const blank: Snapshot = { games: [], mods: [], settings: { theme: startupTheme(), defaultStrategy: 'suffix' }, dataDir: '', monitoring: '', warnings: [] };
const states = { enabled: '已开启', disabled: '已关闭', mixed: '状态不一致', missing: '文件缺失', error: '需要检查' };
type ToggleResult = {id:string;success:boolean;error?:string};
type ToggleReply = {results:ToggleResult[];mods:Mod[];warnings:string[]};
type ToggleProgress = {operationId:string;done:number;total:number;result:ToggleResult;mod:Mod|null};
function IconButton({ label, children, onClick, disabled = false }: { label: string; children: ReactNode; onClick: () => void; disabled?: boolean }) {
  return <button className="icon-button" title={label} aria-label={label} onClick={onClick} disabled={disabled}>{children}</button>;
}
function Modal({ title, children, onClose, beforeClose, wide = false, drawer = false, footer }: { title: string; children: ReactNode; onClose: () => void; beforeClose?:()=>Promise<boolean>; wide?: boolean; drawer?: boolean; footer?: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [closing, setClosing] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeRef = useRef(onClose);
  const guard = useRef(beforeClose); guard.current=beforeClose;
  const checking = useRef(false);
  closeRef.current = onClose;
  const dismiss = useCallback(() => {
    if (timer.current || checking.current) return;
    checking.current=true;
    void (async()=>{
      try { if (guard.current && !await guard.current()) return;
        setClosing(true);
        timer.current = setTimeout(() => closeRef.current(), matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 180);
      } finally {checking.current=false;}
    })().catch(()=>{});
  }, []);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement;
    (ref.current?.querySelector<HTMLElement>('input,textarea,select') || ref.current?.querySelector<HTMLElement>('button'))?.focus();
    const key = (event: KeyboardEvent) => {
      const dialogs=document.querySelectorAll('[role="dialog"]');
      if(dialogs[dialogs.length-1]!==ref.current)return;
      if (event.key === 'Escape') { event.preventDefault(); dismiss(); }
      if (event.key === 'Tab') {
        const nodes = ref.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input,textarea,select,[tabindex="0"]');
        if (!nodes?.length) return;
        const first = nodes[0], last = nodes[nodes.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
        if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
      }
    };
    document.addEventListener('keydown', key);
    return () => { document.removeEventListener('keydown', key); if (timer.current) clearTimeout(timer.current); previous?.focus({preventScroll:true}); };
  }, []);
  return <div className={`overlay ${drawer ? 'drawer-overlay' : ''} ${closing ? 'closing' : ''}`} onMouseDown={e => e.target === e.currentTarget && dismiss()}><div className={`modal ${wide ? 'wide' : ''} ${drawer ? 'detail-drawer' : ''}`} role="dialog" aria-modal="true" aria-label={title} ref={ref} onClickCapture={e => { if ((e.target as HTMLElement).closest('[data-dismiss]')) { e.preventDefault(); e.stopPropagation(); dismiss(); } }}><div className="modal-heading"><h2>{title}</h2><IconButton label="关闭" onClick={dismiss}><X size={19}/></IconButton></div><div className="modal-body">{children}</div>{footer && <div className="modal-footer">{footer}</div>}</div></div>;
}
function WindowBar({ onError }: { onError: (error: string) => void }) {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (!desktop) return;
    const win = getCurrentWindow(); let disposed = false; let off: (() => void) | undefined;
    const sync = () => void win.isMaximized().then(value => { if (!disposed) setMaximized(value); }).catch(() => {});
    sync(); void win.onResized(sync).then(fn => { if (disposed) fn(); else off = fn; });
    return () => { disposed = true; off?.(); };
  }, []);
  const act = (operation: 'minimize' | 'toggleMaximize' | 'close') => {
    if (desktop) void getCurrentWindow()[operation]().catch(e => onError(String(e)));
  };
  return <header className="window-bar"><div className="window-drag-region" data-tauri-drag-region/><div className="window-controls"><button aria-label="最小化" title="最小化" onClick={() => act('minimize')}><Minus size={16}/></button><button aria-label={maximized ? '还原窗口' : '最大化'} title={maximized ? '还原' : '最大化'} onClick={() => act('toggleMaximize')}>{maximized ? <Copy size={13}/> : <Square size={13}/>}</button><button className="window-close" aria-label="关闭窗口" title="关闭" onClick={() => act('close')}><X size={18}/></button></div></header>;
}
function GameImage({ game }: { game: Game }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [game.imagePath]);
  return <span className="game-image">{game.imagePath && !failed ? <img src={imageUrl(game.imagePath)} alt="" onError={() => setFailed(true)}/> : <Gamepad2 size={22}/>}</span>;
}

export default function App() {
  const [data, setData] = useState<Snapshot>(blank);
  const [gameId, setGameId] = useState('');
  const [folderChoices, setFolderChoices] = useState<Record<string,string>>({});
  const [deleteMod, setDeleteMod] = useState<Mod | null>(null);
  const [search, setSearch] = useState('');
  const searchInput = useRef<HTMLInputElement>(null);
  const libraryScroll = useRef<HTMLDivElement>(null);
  const view = useRef<ViewPreferences>({});
  const viewInitialized = useRef(false);
  const viewTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [context, setContext] = useState<{mod:Mod;x:number;y:number} | null>(null);
  const [undoId, setUndoId] = useState<string | null>(null);
  const [errorModId, setErrorModId] = useState<string | null>(null);
  const [errorFolder, setErrorFolder] = useState<{gameId:string;folderId?:string} | null>(null);
  const closeContext = useCallback(() => setContext(null), []);
  const persistView = useCallback(() => {
    if (!desktop || !viewInitialized.current) return;
    if (viewTimer.current) clearTimeout(viewTimer.current);
    viewTimer.current = setTimeout(() => { void request('saveView', {view:view.current}).catch(() => {}); }, 250);
  }, []);
  const [selected, setSelected] = useState<string[]>([]);
  const selectedIds=useMemo(()=>new Set(selected),[selected]);
  const selectRow=useCallback((id:string,checked:boolean)=>setSelected(previous=>checked?[...new Set([...previous,id])]:previous.filter(item=>item!==id)),[]);
  const [selecting, setSelecting] = useState(false);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const snapshotSequence = useRef(0);
  const viewRevision = useRef<number | undefined>(undefined);
  const refreshInFlight = useRef<Promise<void> | null>(null);
  const refreshQueued = useRef(false);
  const pendingTheme = useRef<Theme | null>(null);
  const confirmedTheme = useRef<Theme>(startupTheme());
  const themeSave = useRef<Promise<void> | null>(null);
  const [pendingAction, setPendingAction] = useState('');
  const [togglingIds, setTogglingIds] = useState<string[]>([]);
  const [loading, setLoading] = useState(desktop);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [editor, setEditor] = useState<Game | 'new' | null>(null);
  const [detailId, setDetailId] = useState<string | null>(null);
  const [settings, setSettings] = useState(false);
  const [grouping, setGrouping] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<Game | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const closeGuard = useRef<null|(()=>Promise<boolean>)>(null);
  const registerCloseGuard = useCallback((guard:null|(()=>Promise<boolean>))=>{closeGuard.current=guard;},[]);
  const activeImport = useRef<{id:string;done:Promise<void>}|null>(null);
  const [importProgress,setImportProgress] = useState<ImportProgress|null>(null);
  const [cancelRequested,setCancelRequested] = useState(false);
  const [windowClosing,setWindowClosing] = useState(false);
  const latestToggle=useRef(toggle);latestToggle.current=toggle;
  const toggleRow=useCallback((id:string,enabled:boolean)=>void latestToggle.current([id],enabled),[]);
  const contextRow=useCallback((mod:Mod,x:number,y:number)=>{if(!busyRef.current)setContext({mod,x,y});},[]);
  const game = data.games.find(g => g.id === gameId) || data.games[0];
  const folderChoice = game ? folderChoices[game.id] : undefined;
  const folder = game?.folders.find(f => f.id === folderChoice) || game?.folders[0];
  const history = folderChoice === '__history' && data.mods.some(m => m.gameId === game?.id && m.retired);
  const activeFolderId = history ? '__history' : folder?.id;
  const mods = useMemo(()=>data.mods.filter(m => m.gameId === game?.id && (history ? m.retired : !m.retired && m.folderId === folder?.id)),[data.mods,game?.id,history,folder?.id]);
  const hasHistory = useMemo(()=>data.mods.some(m => m.gameId === game?.id && m.retired),[data.mods,game?.id]);
  const searchIndex = useMemo(()=>new Map(mods.map(m=>[m.id,[modName(m),m.description,...m.members.map(f=>basename(f.originalPath))].join(' ').toLowerCase()])),[mods]);
  const deferredSearch=useDeferredValue(search).toLowerCase();
  const filtered=useMemo(()=>mods.filter(m=>searchIndex.get(m.id)!.includes(deferredSearch)),[mods,searchIndex,deferredSearch]);
  const gameCounts=useMemo(()=>{const counts=new Map<string,{total:number;enabled:number}>();for(const mod of data.mods){if(mod.retired)continue;const count=counts.get(mod.gameId)||{total:0,enabled:0};count.total++;if(mod.state==='enabled')count.enabled++;counts.set(mod.gameId,count);}return counts;},[data.mods]);
  const detail = data.mods.find(m => m.id === detailId);
  const acceptSnapshot = useCallback((next: Snapshot) => {
    viewRevision.current=next.revision;
    if (pendingTheme.current === null) confirmedTheme.current = next.settings.theme;
    setData(pendingTheme.current === null ? next : {...next,settings:{...next.settings,theme:pendingTheme.current}});
  }, []);
  const reload = useCallback(async (force = false):Promise<void> => {
    if (!desktop || (busyRef.current && !force)) return;
    if (!force && refreshInFlight.current) {refreshQueued.current=true;return refreshInFlight.current;}
    const work=(async()=>{
    const sequence = ++snapshotSequence.current;
    const next = await request<Snapshot & {unchanged?:boolean}>(force?'snapshot':'viewSnapshot',force?{}:{revision:viewRevision.current});
    if (sequence!==snapshotSequence.current || (busyRef.current&&!force)) return;
    if(next.unchanged){
      setData(previous=>previous.monitoring===next.monitoring&&JSON.stringify(previous.warnings)===JSON.stringify(next.warnings)?previous:{...previous,monitoring:next.monitoring,warnings:next.warnings});
      return;
    }
    if (!viewInitialized.current) {
      const saved = next.view || {};
      view.current = saved; viewInitialized.current = true;
      setGameId(typeof saved.gameId === 'string' ? saved.gameId : '');
      setFolderChoices(saved.folderChoices || {});
    }
    if (sequence === snapshotSequence.current && (!busyRef.current || force)) {
      acceptSnapshot(next);
    }
    })();
    if(!force)refreshInFlight.current=work;
    try{await work;}finally{
      if(refreshInFlight.current===work){
        refreshInFlight.current=null;
        if(refreshQueued.current){refreshQueued.current=false;queueMicrotask(()=>void reload().catch(e=>setError(String(e))));}
      }
    }
  }, [acceptSnapshot]);
  useEffect(()=>{
    if(!desktop)return;
    let disposed=false;let off:(()=>void)|undefined;
    void listen<ImportProgress>('import-progress',event=>{if(event.payload.operationId===activeImport.current?.id)setImportProgress(event.payload);}).then(fn=>{if(disposed)fn();else off=fn;});
    return()=>{disposed=true;off?.();};
  },[]);
  useEffect(() => {
    if (!desktop) return;
    void reload().catch(e => setError(String(e))).finally(() => setLoading(false));
    let unlisten: (() => void) | undefined;
    let refreshTimer:ReturnType<typeof setTimeout>|undefined;
    void listen('library-changed', () => {clearTimeout(refreshTimer);refreshTimer=setTimeout(()=>void reload().catch(e=>setError(String(e))),100);}).then(fn => { unlisten = fn; });
    // Native watcher events drive changes; this is only a missed-event fallback.
    const timer = window.setInterval(() => {if(!document.hidden)void reload().catch(()=>{});}, 15000);
    const visible=()=>{if(!document.hidden)void reload().catch(()=>{});};document.addEventListener('visibilitychange',visible);
    return () => { clearInterval(timer);clearTimeout(refreshTimer);document.removeEventListener('visibilitychange',visible);unlisten?.(); };
  }, [reload]);
  useLayoutEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)');
    const apply = () => {const dark=applyTheme(data.settings.theme);if(desktop)void invoke('sync_theme',{theme:data.settings.theme,dark,ready:false}).catch(e=>setError(String(e)));};
    apply(); media.addEventListener('change', apply); return () => media.removeEventListener('change', apply);
  }, [data.settings.theme]);
  const startupShown=useRef(false);
  useLayoutEffect(()=>{
    if(!desktop||loading||startupShown.current)return;
    startupShown.current=true;
    // Hidden WebViews may pause animation frames. The layout effect runs after
    // DOM commit without waiting for a hidden-window requestAnimationFrame.
    void invoke('sync_theme',{theme:data.settings.theme,dark:applyTheme(data.settings.theme),ready:true}).catch(e=>{setError(String(e));});
  },[loading,data.settings.theme]);
  useEffect(() => { setSelected([]); setSearch(''); setSelecting(false); setDetailId(null); setContext(null); }, [game?.id, activeFolderId]);
  useEffect(() => {
    if (!game || !viewInitialized.current) return;
    view.current = {...view.current, gameId:game.id, folderChoices}; persistView();
  }, [game?.id, folderChoices, persistView]);
  useLayoutEffect(() => {
    if (settings || !libraryScroll.current) return;
    libraryScroll.current.scrollTop = search ? 0 : Math.max(0, Number(view.current.scrolls?.[`${game?.id}:${activeFolderId}`]) || 0);
  }, [settings, game?.id, activeFolderId, !!search, loading]);
  useEffect(() => {
    const key = (e:KeyboardEvent) => {
      if(document.querySelector('[role="dialog"]'))return;
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f' && !editor && !detailId && !deleteMod && !grouping && !confirmDelete) {
        e.preventDefault(); setSettings(false); requestAnimationFrame(() => searchInput.current?.focus());
      }
      if (e.key === 'Escape' && !editor && !detailId && !deleteMod && !grouping && !confirmDelete && !context) {
        if (search) { e.preventDefault(); setSearch(''); }
        else if (selecting) { setSelected([]); setSelecting(false); }
      }
    };
    document.addEventListener('keydown', key); return () => document.removeEventListener('keydown', key);
  }, [search, selecting, editor, detailId, deleteMod, grouping, confirmDelete, context]);
  useEffect(() => () => { if (viewTimer.current) clearTimeout(viewTimer.current); }, []);
  useEffect(() => {
    if (!desktop) return;
    let disposed=false, closing=false; let off:(()=>void)|undefined;
    const win=getCurrentWindow();
    void win.onCloseRequested(event => {
      event.preventDefault(); if(closing) return; closing=true;setWindowClosing(true);
      if(viewTimer.current) clearTimeout(viewTimer.current);
      void (async()=>{
        if(closeGuard.current && !await closeGuard.current()){closing=false;setWindowClosing(false);return;}
        if(activeImport.current){setCancelRequested(true);await request('cancelImport',{operationId:activeImport.current.id});await activeImport.current?.done;}
        await themeSave.current;
        await request('saveView',{view:view.current});
        await win.destroy();
      })().catch(e=>{closing=false;setWindowClosing(false);setError(`关闭前保存未完成：${String(e)}`);});
    }).then(fn=>{if(disposed)fn();else off=fn;});
    return ()=>{disposed=true;off?.();};
  }, []);
  useEffect(() => { if (notice) { const t = setTimeout(() => setNotice(''), 6500); return () => clearTimeout(t); } }, [notice]);
  useEffect(() => {
    if (!settings) return;
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape' && !document.querySelector('[role="dialog"]') && !editor && !detailId && !grouping && !confirmDelete && !deleteMod) setSettings(false); };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, [settings, editor, detailId, grouping, confirmDelete, deleteMod]);
  function saveAppearance(value: Record<string, unknown>) {
    const theme=value.theme as Theme;
    if (!desktop || !['system','light','dark'].includes(theme)) return;
    pendingTheme.current=theme;
    ++snapshotSequence.current;
    applyTheme(theme);
    setData(previous=>({...previous,settings:{...previous.settings,theme}}));
    setError('');
    if (themeSave.current) return;
    // Serialize writes, coalescing repeated clicks to the latest choice. A slow
    // disk or library scan must never delay the visible appearance change.
    themeSave.current=(async()=>{
      while(pendingTheme.current!==null){
        const saving=pendingTheme.current;
        try{
          const reply=await request<{settings:Snapshot['settings']}>('settings',{theme:saving,compact:true});
          confirmedTheme.current=reply.settings.theme;
        }catch(e){
          setError(`外观设置未能保存：${String(e)}`);
          if(pendingTheme.current===saving){
            applyTheme(confirmedTheme.current);
            setData(previous=>({...previous,settings:{...previous.settings,theme:confirmedTheme.current}}));
          }
        }
        if(pendingTheme.current===saving)pendingTheme.current=null;
      }
      ++snapshotSequence.current; // Reject snapshots started before the final write.
    })().finally(()=>{themeSave.current=null;});
  }
  async function mutate(action: string, args: Record<string, unknown> = {}, success?: string) {
    if (busyRef.current) return false;
    if(action==='openFolder'||action==='openRecycleBin'){
      setError('');setErrorModId(null);
      setErrorFolder(typeof args.gameId==='string'?{gameId:args.gameId,folderId:typeof args.folderId==='string'?args.folderId:undefined}:null);
      try{await request(action,args);return true;}catch(e){setError(String(e));return false;}
    }
    busyRef.current = true; ++snapshotSequence.current;
    setBusy(true); setPendingAction(action); setError(''); setErrorModId(typeof args.id === 'string' ? args.id : null);
    const affected=data.mods.find(m=>m.id===args.id);
    const recovering=data.recentDeletions?.find(r=>r.id===args.operationId);
    const owner=affected?.gameId || recovering?.gameId || (typeof args.gameId==='string' ? args.gameId : undefined);
    setErrorFolder(owner ? {gameId:owner,folderId:affected?.folderId || recovering?.folderId || (typeof args.folderId==='string' ? args.folderId : undefined)} : null);
    try {
      const next=await request<Snapshot & {scope?:{gameId:string;folderId?:string};success?:boolean}>(action,{...args,quiet:true,...(['editMod','scan'].includes(action)?{compact:true}:{})});
      if(next?.games){acceptSnapshot(next);}
      else if(next?.mods){
        viewRevision.current=next.revision;
        const updates=new Map(next.mods.map(m=>[m.id,m]));
        setData(previous=>{
          const retained=previous.mods.filter(m=>!next.scope || m.gameId!==next.scope.gameId || (!!next.scope.folderId&&m.folderId!==next.scope.folderId) || updates.has(m.id));
          const seen=new Set(retained.map(m=>m.id));
          return {...previous,mods:[...retained.map(m=>updates.get(m.id)||m),...next.mods.filter(m=>!seen.has(m.id))],warnings:next.warnings,monitoring:next.monitoring||previous.monitoring};
        });
      }else if(!next.success)await reload(true);
      if(success)setNotice(success);return true;
    }
    catch (e) { await reload(true).catch(() => {}); setError(String(e)); return false; }
    finally { busyRef.current = false; setBusy(false); setPendingAction(''); }
  }
  async function toggle(ids: string[], enabled: boolean) {
    if (busyRef.current || !ids.length) return;
    if (!enabled && ids.some(id => data.games.find(g => g.id === data.mods.find(m => m.id === id)?.gameId)?.strategy === 'move')) { setError('请先开启该游戏所有目录中的旧版 MOD，再编辑并保存游戏，切换到后缀启停。'); return; }
    busyRef.current = true; ++snapshotSequence.current;
    setBusy(true); setTogglingIds(ids); setError(''); setErrorModId(ids[0]); setNotice('');
    if (ids.length > 1) setProgress({ done: 0, total: ids.length });
    const failures: string[] = []; let successes = 0;
    const operationId=crypto.randomUUID();
    const completed=new Map<string,ToggleResult>();
    let off:(()=>void)|undefined;
    let active=true;
    let interrupted=false;
    const applyMods=(mods:Mod[],warnings?:string[])=>{
      const updates=new Map(mods.map(mod=>[mod.id,mod]));
      setData(previous=>({...previous,mods:previous.mods.map(mod=>updates.get(mod.id)||mod),...(warnings?{warnings}:{})}));
    };
    try {
      off=await listen<ToggleProgress>('toggle-progress',event=>{
        const update=event.payload;
        if(!active||update.operationId!==operationId)return;
        completed.set(update.result.id,update.result);
        if(update.mod)applyMods([update.mod]);
        setTogglingIds(previous=>previous.filter(id=>id!==update.result.id));
        if(ids.length>1)setProgress({done:update.done,total:update.total});
      });
      const result=await request<ToggleReply>('toggle',{ids,enabled,operationId});
      applyMods(result.mods,result.warnings);
      for(const r of result.results){const item=data.mods.find(m=>m.id===r.id);if(r.success)successes++;else{setErrorModId(r.id);failures.push(`${item?modName(item):'MOD'}：${r.error||'操作失败'}`);}}
      if (ids.length > 1) setNotice(`${successes} 个 MOD 已${enabled ? '开启' : '关闭'}${failures.length ? `，${failures.length} 个失败` : ''}`);
      if (failures.length) setError(failures.join('\n'));
    } catch (e) { interrupted=true;setError(String(e)); successes=[...completed.values()].filter(r=>r.success).length;if (successes) setNotice(`${successes} 个 MOD 已${enabled ? '开启' : '关闭'}，其余操作中断，请检查结果`); }
    finally {
      active=false;off?.();busyRef.current = false; setBusy(false); setTogglingIds([]); setProgress(null);
      // Verified patches include failed items too. The watcher refreshes external
      // changes; avoid queuing a full scan ahead of the user's next switch click.
      if(interrupted)void reload().catch(()=>{});
    }
  }
  async function backup(importing: boolean) {
    try {
      const path = importing ? await open({ title: '导入管理器备份', filters: [{ name: '管理器备份', extensions: ['json'] }] }) : await save({ title: '导出管理器备份', defaultPath: 'Mod-Manager-backup.json', filters: [{ name: '管理器备份', extensions: ['json'] }] });
      if (typeof path === 'string') await mutate(importing ? 'import' : 'export', { path }, importing ? '备份已导入，真实 MOD 文件未被修改' : '管理器配置、备注和图片已导出');
    } catch (e) { setError(String(e)); }
  }
  async function importDroppedFiles(paths:string[], destination:DropTarget) {
    if (busyRef.current) return;
    busyRef.current=true; ++snapshotSequence.current;
    setBusy(true);setPendingAction('importFiles');setContext(null);setError('');setNotice('');setErrorModId(null);
    setErrorFolder({gameId:destination.gameId,folderId:destination.folderId});
    const operationId=crypto.randomUUID();let finish!:()=>void;
    activeImport.current={id:operationId,done:new Promise<void>(resolve=>{finish=resolve;})};
    setCancelRequested(false);setImportProgress({operationId,phase:'preparing',copiedBytes:0,totalBytes:0,currentName:'',index:0,count:paths.length});
    try {
      const result=await request<{results:{name:string;success:boolean;cancelled?:boolean;error?:string}[];scanError?:string;batchError?:string;snapshot:Snapshot}>('importFiles',{operationId,gameId:destination.gameId,folderId:destination.folderId,expectedPath:destination.path,paths});
      acceptSnapshot(result.snapshot);
      const successes=result.results.filter(r=>r.success).length, failures=result.results.filter(r=>!r.success&&!r.cancelled),cancelled=result.results.filter(r=>r.cancelled).length;
      if(successes||cancelled)setNotice(`${successes?`已添加 ${successes} 个项目到「${destination.folderName}」`:'导入已取消'}${cancelled&&successes?`，${cancelled} 个已取消`:''}${failures.length?`，${failures.length} 个失败`:''}`);
      const errors=failures.map(r=>`${r.name}：${r.error||'导入失败'}`);
      if(result.batchError)errors.push(result.batchError);
      if(result.scanError)errors.push(`文件已核对，但刷新列表失败：${result.scanError}`);
      if(errors.length)setError(errors.join('\n'));
    } catch(e) {setError(String(e));await reload(true).catch(()=>{});}
    finally {finish();activeImport.current=null;busyRef.current=false;setBusy(false);setPendingAction('');setImportProgress(null);setCancelRequested(false);}
  }
  async function cancelImport(){if(!activeImport.current||cancelRequested)return;setCancelRequested(true);try{await request('cancelImport',{operationId:activeImport.current.id});}catch(e){setCancelRequested(false);setError(String(e));}}
  const errorMod = data.mods.find(m => m.id === errorModId);
  async function reveal(mod:Mod) { if (mod.members[0]) await mutate('revealMember',{id:mod.id,memberId:mod.members[0].id}); }
  async function copyPaths(mod:Mod) {
    try { await navigator.clipboard.writeText(mod.members.map(f => displayPath(f.currentPath)).join('\n')); setNotice('路径已复制'); }
    catch { setErrorModId(mod.id); setError('无法写入剪贴板，请在详情中选择并复制文件路径'); }
  }
  async function undo(operationId:string) { if (await mutate('restoreMod',{operationId},'文件与管理信息已恢复')) setUndoId(null); }
  useEffect(() => { if (!undoId) return; const timer=setTimeout(() => setUndoId(null),12000); return () => clearTimeout(timer); }, [undoId]);
  const closeEditor = useCallback(() => setEditor(null), []);
  const closeSettings = useCallback(() => setSettings(false), []);
  const closeGroup = useCallback(() => setGrouping(false), []);
  const closeDelete = useCallback(() => setConfirmDelete(null), []);
  const closeDetail = useCallback(() => setDetailId(null), []);
  const closeModDelete = useCallback(() => setDeleteMod(null), []);
  return <div className="app-shell"><WindowBar onError={setError}/>
    <aside className="sidebar"><div className="brand"><span className="brand-mark"><img src={appIconUrl} alt=""/></span><span>MOD Manager<small>Simple. Clean. Effortless.</small></span></div>
      <button className="add-game primary" onClick={() => setEditor('new')} disabled={!desktop || busy}><Plus size={19}/>添加游戏</button><div className="section-label">游戏库 {data.games.length > 0 && <span>{data.games.length}</span>}</div>
      <nav className="games" aria-label="游戏库">{data.games.map(g => { const count = gameCounts.get(g.id) || {total:0,enabled:0}; return <button key={g.id} className={`game-item ${!settings && game?.id === g.id ? 'active' : ''}`} onClick={() => { setSettings(false); setGameId(g.id); }} title={g.name}><GameImage game={g}/><span className="game-info"><strong>{g.name}</strong><small title="所有目录合计">共 {count.total} 个 MOD · {count.enabled} 已开启</small></span></button>; })}</nav>
      <div className="sidebar-bottom"><button className={`settings-button ${settings ? 'active' : ''}`} onClick={() => { setSettings(true); setDetailId(null); setSelected([]); setSelecting(false); }}><Settings size={20}/>设置<ChevronRight size={15}/></button></div>
    </aside>
    <main className={`main-content ${settings ? 'settings-content' : game && !loading ? 'library-content' : ''}`}><div className="page-view" key={settings ? 'settings' : game?.id || 'welcome'}>
      {settings ? <SettingsPage data={data} busy={busy} onBack={closeSettings} onSettings={saveAppearance} onBackup={value => void backup(value)} onRestore={id => void undo(id)} onRecycleBin={() => void mutate('openRecycleBin')} onResolveImport={(id,cleanup)=>mutate('resolveImport',{id,cleanup,confirmed:true},'导入记录已核对')} onRefresh={() => void mutate('scan')} onFolder={() => void mutate('openFolder')}/> : loading ? <div className="empty-state"><LoaderCircle className="spin" size={30}/><h2>正在读取游戏库</h2></div> : !game ? <div className="welcome"><span className="welcome-icon"><Gamepad2 size={44} strokeWidth={1.3}/></span><p className="eyebrow">LESS MANAGING. MORE PLAYING.</p><h1>让每一个 MOD，井然有序。</h1><p>为不同游戏集中管理 MOD，记录用途与备注。<br/>一键启停，把更多时间留给游戏。</p><button className="primary" onClick={() => setEditor('new')} disabled={!desktop}><Plus size={17}/>添加第一个游戏</button><div className="welcome-foot"><ShieldCheck size={15}/>{desktop ? '管理配置保存在本机，MOD 文件始终由你掌控。' : '这是浏览器预览。请在 Windows 桌面程序中添加游戏和管理真实文件。'}</div></div> : <>
      <div className="library-header"><section className="game-hero"><GameImage game={game}/><div className="hero-content"><div className="page-header"><div><h1 title={game.name}>{game.name}</h1><button className="folder-link" title={history ? '旧目录历史记录' : displayPath(folder?.path || game.modDir)} onClick={() => void mutate('openFolder', { gameId: game.id, folderId: folder?.id })} disabled={busy || history}><span>{history ? '旧目录历史记录 · 不会操作真实文件' : displayPath(folder?.path || game.modDir)}</span></button></div><button className="secondary open-folder" onClick={() => void mutate('openFolder', { gameId: game.id, folderId: folder?.id })} disabled={busy || history}><FolderOpen size={19}/>打开文件夹</button></div><div className="summary-line"><small className="count-scope">{history ? '历史记录' : '当前目录'}</small><span><Box size={18}/><strong>{mods.length}</strong> 个 MOD</span><span><Play size={17}/><strong>{mods.filter(m => m.state === 'enabled').length}</strong> 个已开启</span><span className="strategy-chip" title="是否停止加载取决于游戏，详情中可核对文件状态"><Settings size={17}/>{game.strategy === 'suffix' ? '后缀禁用 .disabled' : '旧版禁用 · 请先恢复'}</span></div></div></section>
      <div className="toolbar"><DirectoryPicker key={game.id} folders={game.folders} value={activeFolderId || ''} hasHistory={hasHistory} disabled={busy} onChange={id => setFolderChoices({...folderChoices,[game.id]:id})}/><label className="search"><Search size={21}/><input ref={searchInput} aria-label="搜索名称" value={search} onChange={e => setSearch(e.target.value)} placeholder="搜索名称"/>{search && <IconButton label="清空搜索" onClick={() => setSearch('')}><X size={15}/></IconButton>}</label><button className="secondary refresh-button" onClick={() => void mutate('scan', { gameId: game.id, folderId: folder?.id })} disabled={busy} aria-label="刷新目录"><RefreshCw size={18} className={pendingAction === 'scan' ? 'spin' : ''}/><span>手动刷新</span></button><button className={`secondary ${selecting ? 'chosen' : ''}`} onClick={() => { setSelecting(!selecting); setSelected([]); }} disabled={busy}><CheckCheck size={16}/>{selecting ? '完成选择' : '批量管理'}</button><IconButton label="编辑游戏与目录" onClick={() => setEditor(game)} disabled={busy}><Settings size={19}/></IconButton></div>
      {search.trim() && <div className="search-results" role="status">找到 {filtered.length} / {mods.length} 个 MOD</div>}
      </div><div className="library-scroll" key={activeFolderId} ref={libraryScroll} onScroll={e => { if (!search) { view.current={...view.current,scrolls:{...view.current.scrolls,[`${game?.id}:${activeFolderId}`]:e.currentTarget.scrollTop}}; persistView(); } }}>
      {selecting && <div className="selection-heading"><label><input type="checkbox" checked={filtered.length > 0 && filtered.every(m => selected.includes(m.id))} onChange={e => setSelected(e.target.checked ? filtered.map(m => m.id) : [])}/>选择当前结果</label><span>{selected.length} 个已选择</span></div>}
      <div className="mod-list" aria-busy={search.toLowerCase()!==deferredSearch}>{filtered.map(mod => <ModListRow key={mod.id} mod={mod} selected={selectedIds.has(mod.id)} focused={detailId===mod.id} selecting={selecting} busy={busy} toggling={togglingIds.includes(mod.id)} onSelect={selectRow} onOpen={setDetailId} onToggle={toggleRow} onContext={contextRow}/>)}</div>
      {!filtered.length && <div className="empty-state"><FolderOpen size={35} strokeWidth={1.2}/><h2>{search ? '没有找到匹配的 MOD' : '这里准备好了，等待你的 MOD'}</h2><p>{search ? '试试其他名称、文件名或备注。' : '将文件放入游戏的 MOD 目录，它们会自动出现在这里。'}</p>{!search && <button className="secondary" onClick={() => void mutate('openFolder', { gameId: game.id, folderId: folder?.id })}><FolderOpen size={16}/>打开 MOD 文件夹</button>}</div>}
      <div className="content-footer"><span><span className="monitor-dot"/>{data.monitoring || '监控准备中'}</span><span>文件状态以磁盘核对结果为准</span></div>
      </div></>}
      {data.warnings.length > 0 && <div className="warning-banner"><AlertCircle size={17}/><span>{data.warnings.join(' · ')}</span></div>}
    </div></main>
    <DropImportOverlay target={game&&folder&&!history?{gameId:game.id,folderId:folder.id,gameName:game.name,folderName:folder.name,path:folder.path}:null} blocked={busy?'请等待当前操作完成':loading?'正在读取游戏库，请稍候':editor||detailId||deleteMod||grouping||confirmDelete?'请先关闭当前弹窗':settings?'请返回游戏的 MOD 页面后拖入文件':history?'历史记录不能添加文件，请选择实际 MOD 目录':!game||!folder?'请先添加游戏并选择 MOD 目录':''} progress={importProgress} cancelRequested={cancelRequested} onCancel={()=>void cancelImport()} onImport={importDroppedFiles} onError={setError}/>
    {selected.length > 0 && <div className="batch-bar"><strong>已选 {selected.length} 个 MOD</strong><button onClick={() => void toggle(selected, true)} disabled={busy}>开启</button><button onClick={() => void toggle(selected, false)} disabled={busy}>关闭</button><button onClick={() => setGrouping(true)} disabled={busy || selected.length < 2}><Layers size={15}/>创建分组</button><IconButton label="取消选择" onClick={() => setSelected([])} disabled={busy}><X size={17}/></IconButton></div>}
    {progress && <div className="progress-toast" role="status"><LoaderCircle size={17} className="spin"/>正在处理 {progress.done} / {progress.total}<progress value={progress.done} max={progress.total}/></div>}
    {notice && <div className="toast" role="status"><Check size={17}/>{notice}<IconButton label="关闭提示" onClick={() => setNotice('')}><X size={15}/></IconButton></div>}
    {context && <ModContextMenu {...context} busy={busy} onClose={closeContext} onDetails={() => setDetailId(context.mod.id)} onReveal={() => void reveal(context.mod)} onCopy={() => void copyPaths(context.mod)} onDelete={() => {setDeleteMod(context.mod);setDetailId(null);}}/>}
    {undoId && <div className="undo-toast" role="status"><Trash2 size={16}/><span>MOD 已移入回收站</span><button disabled={busy} onClick={() => void undo(undoId)}>撤销删除</button><IconButton label="关闭撤销提示" onClick={() => setUndoId(null)}><X size={14}/></IconButton></div>}
    {error && <div className="error-toast" role="alert"><div><AlertCircle size={18}/><strong>操作未完成</strong><IconButton label="关闭错误" onClick={() => setError('')}><X size={16}/></IconButton></div><p>{error.split('\n')[0].slice(0, 220)}{error.split('\n')[0].length > 220 ? '…' : ''}</p><p className="error-help">{/正在运行|退出游戏/.test(error) && (errorMod || errorFolder) ? `请先退出「${data.games.find(g=>g.id === (errorMod?.gameId || errorFolder?.gameId))?.name || "对应游戏"}」，再重试操作。` : errorAdvice(error)}</p><div className="error-actions">{errorMod && <button disabled={busy} onClick={() => {setDetailId(errorMod.id);setError('');}}>核对成员</button>}{errorMod ? <button disabled={busy} onClick={() => void reveal(errorMod)}>定位文件</button> : errorFolder && <button disabled={busy} onClick={() => void mutate("openFolder",errorFolder)}>打开所在文件夹</button>}{/回收站/.test(error) && <button disabled={busy} onClick={() => void mutate('openRecycleBin')}>打开回收站</button>}</div>{(error.includes('\n') || error.length > 220) && <details><summary>查看详细结果</summary><pre>{error}</pre></details>}</div>}
    {editor && <GameEditor value={editor === 'new' ? undefined : editor} defaultStrategy={data.settings.defaultStrategy} busy={busy} registerCloseGuard={registerCloseGuard} onClose={closeEditor} onSave={value=>mutate('saveGame',{game:value},'游戏配置已保存')} onDelete={editor !== 'new' ? () => { setConfirmDelete(editor); setEditor(null); } : undefined} onError={setError}/>}
    {detail && <ModDetail key={detail.id} registerCloseGuard={registerCloseGuard} mod={detail} mods={mods} busy={busy||windowClosing} toggling={togglingIds.includes(detail.id)} onClose={closeDetail} onToggle={() => void toggle([detail.id], detail.state !== 'enabled')} onUnify={enabled => void toggle([detail.id], enabled)} onImage={(path, originalPath) => mutate('editMod', { id: detail.id, imagePath: path, originalImagePath: originalPath ?? null }, path ? 'MOD 封面已更新' : 'MOD 封面已移除')} onError={setError} onRename={(memberId,name) => mutate('renameMember', {id:detail.id,memberId,name}, '真实名称已修改')} onDelete={() => { setDeleteMod(detail); setDetailId(null); }} onSave={(description, managed) => mutate('editMod', { id: detail.id, description, managed }, undefined)} onAcceptReplacement={() => mutate('acceptReplacement', { id: detail.id, confirmed: true }, '当前文件已重新关联，名称和备注保留')} onMembers={ids => mutate('setMembers', { id: detail.id, memberIds: ids }, '成员已更新')} onAdd={async ids => {const snapshot=await request('snapshot');const source=snapshot.mods.find(m=>m.id===detail.id);return mutate('group', { ids: [detail.id, ...ids], name: source?.name || detail.name, description: source?.description ?? detail.description }, '已加入分组');}} onUngroup={async () => { if (await mutate('ungroup', { id: detail.id }, '分组已解散，文件未被修改')) setDetailId(null); }}/>}
    {grouping && <GroupEditor busy={busy} count={selected.length} onClose={closeGroup} onSave={async (name, description) => { if (await mutate('group', { ids: selected, name, description }, '逻辑 MOD 分组已创建')) { setGrouping(false); setSelected([]); } }}/>}
    {deleteMod && <DeleteModDialog mod={deleteMod} busy={busy} onClose={closeModDelete} onConfirm={async recordOnly => { if (await mutate(recordOnly ? 'forgetMod' : 'deleteMod', { id:deleteMod.id, confirmed:true, expectedMembers:deleteMod.members.map(f=>({id:f.id,currentPath:f.currentPath,exists:f.exists})) }, recordOnly ? '管理记录已移除' : undefined)) { if (!recordOnly) { const snap=await request('snapshot'); acceptSnapshot(snap); setUndoId(snap.recentDeletions?.find(r => r.name === modName(deleteMod))?.id || snap.recentDeletions?.[0]?.id || null); } setDeleteMod(null); } }}/>}
    {confirmDelete && <Modal title="删除游戏配置？" onClose={closeDelete}><p className="dialog-copy">将移除「{confirmDelete.name}」的管理器配置、分组与备注。实际游戏和 MOD 文件会完整保留，包括当前禁用的文件；建议先开启需要恢复的 MOD。</p><div className="dialog-actions"><button className="secondary" onClick={closeDelete}>取消</button><button className="danger" disabled={busy} onClick={async () => { if (await mutate('deleteGame', { id: confirmDelete.id }, '游戏配置已删除，实际文件保留')) setConfirmDelete(null); }}>删除配置</button></div></Modal>}
  </div>;
}

function SettingsPage({ data, busy, onRestore, onResolveImport, onRecycleBin, onBack, onSettings, onBackup, onRefresh, onFolder }: { data: Snapshot; busy: boolean; onResolveImport:(id:string,cleanup:boolean)=>Promise<boolean>; onRestore: (id:string) => void; onRecycleBin: () => void; onBack: () => void; onSettings: (value: Record<string, unknown>) => void; onBackup: (importing: boolean) => void; onRefresh: () => void; onFolder: () => void }) {
  const [resolution,setResolution]=useState<PendingImport|null>(null);
  return <section className="settings-page"><header className="settings-heading"><IconButton label="返回游戏库" onClick={onBack}><ArrowLeft size={20}/></IconButton><div><h1>设置</h1><p>自定义 MOD Manager 的外观和数据管理选项。</p></div><span aria-hidden="true"/></header><div className="settings-scroll"><div className="settings-grid">
    <section className="settings-card"><div className="card-heading"><Monitor size={23}/><div><h2>外观</h2><p>选择舒适的主题外观。</p></div></div><div className="theme-options">{([['system', Monitor, '跟随系统'], ['light', Sun, '浅色'], ['dark', Moon, '深色']] as const).map(([value, Icon, label]) => <button key={value} disabled={!desktop || busy} aria-pressed={data.settings.theme === value} className={data.settings.theme === value ? 'active' : ''} onClick={() => onSettings({ ...data.settings, theme: value as Theme })}><Icon size={23}/>{label}<span className="radio-dot"/></button>)}</div></section>
    <section className="settings-card"><div className="card-heading"><Folder size={23}/><div><h2>文件监控状态</h2><p>自动同步 MOD 目录中的文件变化。</p></div></div><div className="settings-monitor"><span className="monitor-badge"><span className="monitor-dot"/>{data.monitoring || '请在桌面程序中使用'}</span><button className="secondary" disabled={!desktop || busy} onClick={onRefresh}><RefreshCw size={17}/>刷新全部</button></div><p>已配置 {data.games.length} 个游戏；目录不可用时会显示提示。</p></section>
    <section className="settings-card full"><div className="card-heading"><Database size={23}/><div><h2>数据管理</h2><p>备份游戏配置、文件关联、备注、分组与图片。</p></div></div><div className="backup-grid"><div><div><h3>导出数据</h3><p>保存管理信息，不包含 MOD 文件本身。</p></div><button className="secondary" disabled={!desktop || busy} onClick={() => onBackup(false)}><Download size={16}/>导出备份</button></div><div><div><h3>导入数据</h3><p>检查格式与冲突，不改变真实 MOD 文件。</p></div><button className="secondary" disabled={!desktop || busy} onClick={() => onBackup(true)}><Upload size={16}/>导入备份</button></div></div></section>
    <section className="settings-card full"><div className="card-heading"><FolderOpen size={23}/><div><h2>数据存储位置</h2><p>配置、SQLite 数据库与本地图片保存在此处。</p></div></div><button className="data-path" disabled={!desktop || busy} onClick={onFolder} title={displayPath(data.dataDir)}><span>{displayPath(data.dataDir) || '桌面程序中可查看数据目录'}</span><span><FolderOpen size={17}/>打开文件夹</span></button></section>
    {!!data.pendingImports?.length && <section className="settings-card full"><div className="card-heading"><AlertCircle size={23}/><div><h2>未完成的导入</h2><p>核对中断的复制；原始文件保持原样。</p></div></div><div className="recent-deletions">{data.pendingImports.map(item=><div key={item.id}><span><strong>{basename(item.source)}</strong><small>{item.targetExists?'目标已有文件，请核对是否完整':'尚未确认导入完成'}</small></span><button className="secondary" disabled={busy} onClick={()=>setResolution(item)}>核对处理</button></div>)}</div></section>}
    {!!data.recentDeletions?.length && <section className="settings-card full"><div className="card-heading"><Trash2 size={23}/><div><h2>最近删除</h2><p>恢复原文件、备注、封面与分组；同名冲突时不会覆盖。</p></div></div><div className="recent-deletions">{data.recentDeletions.map(item => <div key={item.id}><span><strong>{item.name}</strong><small>{new Date(item.at.replace(" ","T")+"Z").toLocaleString("zh-CN",{month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit"})} · {item.status === 'restoring' ? '恢复未完成，可重试' : '回收站记录'}</small></span><button className="secondary" disabled={busy} onClick={() => onRestore(item.id)}>恢复</button></div>)}</div><button className="text-button" onClick={onRecycleBin}>打开 Windows 回收站</button></section>}
    <section className="settings-card full about-card"><div className="card-heading"><Info size={23}/><h2>关于</h2></div><div className="about-line"><span className="brand-mark"><img src={appIconUrl} alt=""/></span><span><strong>MOD Manager {appVersion}</strong><small>让 MOD 的管理和切换更简单。</small></span><span className="about-local"><ShieldCheck size={16}/>完全本地 · 无需登录</span></div></section>
  </div></div>{resolution&&<Modal title="处理未完成的导入" onClose={()=>setResolution(null)}><p className="dialog-copy">仅处理复制记录与管理器创建的暂存副本，不修改来源文件和目标目录中的文件。</p><div className="import-review-paths"><label>来源<span>{displayPath(resolution.source)}</span></label><label>目标<span>{displayPath(resolution.target)}</span><small>{resolution.targetExists?'目标已有文件，请先核对':'目标文件不存在'}</small></label><label>暂存<span>{displayPath(resolution.stage)}</span><small>{resolution.stageExists?'暂存副本仍在磁盘上':'暂存副本不存在'}</small></label></div>{!resolution.cleanupAvailable&&resolution.stageExists&&<p className="dialog-copy">此记录缺少可信清单，不能自动清理。可保留副本并结束记录。</p>}<div className="dialog-actions"><button className="secondary" data-dismiss>取消</button><button className="secondary" disabled={busy} onClick={async()=>{if(await onResolveImport(resolution.id,false))setResolution(null);}}>保留副本并结束</button>{resolution.cleanupAvailable&&<button className="primary" disabled={busy} onClick={async()=>{if(await onResolveImport(resolution.id,true))setResolution(null);}}>清理暂存并结束</button>}</div></Modal>}</section>;
}

function GameEditor({ value, busy, onClose, onSave, onDelete, onError, registerCloseGuard }: { value?: Game; defaultStrategy: Strategy; busy: boolean; onClose: () => void; onSave: (game: Omit<Game, 'id' | 'imagePath'> & { id?: string; imagePath?: string | null }) => Promise<boolean>; registerCloseGuard:(guard:null|(()=>Promise<boolean>))=>void; onDelete?: () => void; onError: (error: string) => void }) {
  const [form, setForm] = useState({ id: value?.id, name: value?.name || '', modDir: value?.modDir ? displayPath(value.modDir) : '', folders: value?.folders.map(f => ({...f,path:displayPath(f.path)})) || [{id:'',name:'MOD',path:''}], exePath: value?.exePath ? displayPath(value.exePath) : '', imagePath: value?.imagePath || '', strategy: 'suffix' as Strategy });
  const [imageRemoved, setImageRemoved] = useState(false);
  const [cropSource,setCropSource]=useState<string|null>(null);
  const initial=useRef(JSON.stringify(form));
  const latestForm=useRef({form,imageRemoved});latestForm.current={form,imageRemoved};
  const [discardPrompt,setDiscardPrompt]=useState(false);
  const discardResolver=useRef<null|((allow:boolean)=>void)>(null);
  const saveConfig=useRef(async()=>false);
  saveConfig.current=async()=>{const ok=await onSave({...form,modDir:form.folders[0].path,folders:form.folders.map(f=>({...f,name:f.name.trim()})),name:form.name.trim(),exePath:form.exePath||null,imagePath:form.imagePath||(imageRemoved?null:undefined)});if(ok){discardResolver.current?.(true);discardResolver.current=null;setDiscardPrompt(false);onClose();}return ok;};
  const guard=useCallback(async()=>{
    if(initial.current===JSON.stringify(latestForm.current.form)&&!latestForm.current.imageRemoved)return true;
    if(discardResolver.current)return false;
    setDiscardPrompt(true);return new Promise<boolean>(resolve=>{discardResolver.current=resolve;});
  },[]);
  const decide=(allow:boolean)=>{setDiscardPrompt(false);discardResolver.current?.(allow);discardResolver.current=null;};
  useEffect(()=>{registerCloseGuard(guard);return()=>{registerCloseGuard(null);discardResolver.current?.(false);};},[guard,registerCloseGuard]);
  const [preview, setPreview] = useState<string | null>(null);
  const [previewError, setPreviewError] = useState(false);
  useEffect(() => {
    let active = true;
    setPreview(null); setPreviewError(false);
    if (form.imagePath && desktop) void request<{url:string}>('previewImage', {path:form.imagePath}).then(result => { if (active) setPreview(result.url); }).catch(() => { if (active) setPreviewError(true); });
    return () => { active = false; };
  }, [form.imagePath]);
  async function pick(field: 'modDir' | 'exePath' | 'imagePath') {
    try { const result = await open({ title: field === 'modDir' ? '选择 MOD 目录' : field === 'exePath' ? '选择游戏可执行文件' : '选择游戏图片', directory: field === 'modDir', filters: field === 'exePath' ? [{ name: '游戏程序', extensions: ['exe'] }] : field === 'imagePath' ? [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'webp'] }] : undefined }); if (typeof result === 'string') { if(field==='imagePath'){const image=await request<{url:string}>('previewImage',{path:result});setCropSource(image.url);}else setForm(f => ({ ...f, [field]: result })); } } catch (e) { onError(String(e)); }
  }
  return <><Modal title={value ? '编辑游戏' : '添加游戏'} onClose={onClose} beforeClose={guard} footer={<div className="dialog-actions">{onDelete && <button type="button" className="text-danger" disabled={busy} onClick={onDelete}><Trash2 size={15}/>删除配置</button>}<button type="button" className="secondary" data-dismiss onClick={onClose}>取消</button><button className="primary" form="game-editor-form" disabled={busy || !form.name.trim() || form.folders.some(f => !f.name.trim() || !f.path.trim())}>{busy && <LoaderCircle className="spin" size={15}/>}保存游戏</button></div>}><p className="dialog-copy">选择游戏的 MOD 目录，填写基本信息即可开始管理。</p><form id="game-editor-form" onSubmit={e => { e.preventDefault(); void saveConfig.current(); }}><label className="field">游戏名称<input required autoFocus maxLength={160} value={form.name} onChange={e => setForm({ ...form, name: e.target.value })} placeholder="例如：Stellar Blade"/></label><FolderEditor folders={form.folders} busy={busy} onChange={folders => setForm({...form,folders})} onError={onError}/><div className="two-fields"><div className="field game-picture-field"><span>游戏图片 <small>可选</small></span><div className="game-picture-picker"><ImageSourcePicker className="picture-preview-source" label="游戏图片来源" disabled={busy} onFile={() => pick('imagePath')} onImage={setCropSource}>{preview ? <img src={preview} alt="游戏图片预览" onError={() => { setPreview(null); setPreviewError(true); }}/> : <Gamepad2 size={31}/>}</ImageSourcePicker><div className="picture-picker-info"><div className="picture-picker-buttons"><button type="button" className="secondary" disabled={busy} onClick={() => void pick('imagePath')}><Upload size={16}/>{form.imagePath ? '更换图片' : '选择图片'}</button>{form.imagePath && <button type="button" className="secondary" disabled={busy} onClick={() => { setForm(f => ({...f,imagePath:''})); setImageRemoved(true); }}><Trash2 size={16}/>移除</button>}</div><small>PNG / JPG / WebP · 最大 20 MB<br/>选择后可拖动、缩放并裁切为方形。</small>{form.imagePath && <span className="picture-filename" title={displayPath(form.imagePath)}>{basename(form.imagePath)}</span>}{previewError && <span className="picture-error">无法预览此图片，请重新选择。</span>}</div></div></div><label className="field">游戏 EXE <small>可选</small><div className="path-field"><input value={form.exePath} onChange={e => setForm({ ...form, exePath: e.target.value })} placeholder="检测游戏运行状态"/><button type="button" className="icon-button" onClick={() => void pick('exePath')} aria-label="选择游戏 EXE"><FolderOpen size={17}/></button></div></label></div><div className="info-note"><CircleHelp size={17}/><span>关闭时为文件追加 .disabled；文件夹保持原位，内部文件递归处理，仅恢复由管理器禁用的内部文件。是否停止加载取决于游戏，请先实际验证。{value?.strategy === 'move' && '旧版移出的 MOD 请先全部开启，再保存游戏以切换到后缀方式。'}</span></div></form></Modal>
  {discardPrompt&&<Modal title="保存游戏配置？" onClose={()=>decide(false)}><p className="dialog-copy">配置已修改。保存后离开，或放弃本次修改。</p><div className="dialog-actions"><button className="secondary" onClick={()=>decide(false)}>继续编辑</button><button className="secondary" onClick={()=>decide(true)}>放弃修改</button><button className="primary" disabled={busy||!form.name.trim()||form.folders.some(f=>!f.name.trim()||!f.path.trim())} onClick={async()=>{if(await saveConfig.current())decide(true);}}>保存并关闭</button></div></Modal>}
  {cropSource&&<ImageCropper source={cropSource} onCancel={()=>setCropSource(null)} onError={onError} onConfirm={async dataUrl=>{const result=await request<{path:string}>('saveCrop',{dataUrl});setForm(f=>({...f,imagePath:result.path}));setImageRemoved(false);setCropSource(null);}}/>}</>;
}
function ModDetail({ mod, mods, registerCloseGuard, busy, toggling, onClose, onDelete, onSave, onMembers, onAdd, onUngroup, onAcceptReplacement, onToggle, onUnify, onImage, onError, onRename }: { registerCloseGuard:(guard:null|(()=>Promise<boolean>))=>void; onDelete: () => void; onToggle: () => void; onUnify:(enabled:boolean)=>void; onImage: (path: string | null, originalPath?:string|null) => Promise<boolean>; onError: (error: string) => void; mod: Mod; mods: Mod[]; busy: boolean; toggling: boolean; onClose: () => void; onRename: (memberId: string, name: string) => Promise<boolean>; onSave: (description: string, managed: boolean) => Promise<boolean>; onMembers: (ids: string[]) => Promise<boolean>; onAdd: (ids: string[]) => Promise<boolean>; onUngroup: () => Promise<void>; onAcceptReplacement: () => Promise<boolean> }) {
  const [note,setNote]=useState({value:mod.description,dirty:false,error:''});
  const [cropSource,setCropSource]=useState<string|null>(null);
  const [originalView,setOriginalView]=useState<string|null>(null);
  const [originalSize,setOriginalSize]=useState(false);
  const [imageReading,setImageReading]=useState(false);
  const imageLock=useRef(false);
  const imageAlive=useRef(true);
  useEffect(()=>{imageAlive.current=true;return()=>{imageAlive.current=false;};},[]);
  async function pasteCover() {
    if(imageLock.current||busy)return;
    imageLock.current=true;setImageReading(true);
    try{const image=await request<{url:string}>('clipboardImage');if(imageAlive.current)setCropSource(image.url);}catch(e){if(imageAlive.current)onError(String(e));}
    finally{imageLock.current=false;if(imageAlive.current)setImageReading(false);}
  }
  async function viewOriginal() {
    if(!mod.originalImagePath){onError('这张旧封面未保存原图，请重新导入图片后查看原图');return;}
    if(imageLock.current)return;
    imageLock.current=true;setImageReading(true);
    try{const image=await request<{url:string}>('previewImage',{path:mod.originalImagePath});if(imageAlive.current){setOriginalSize(false);setOriginalView(image.url);}}catch(e){if(imageAlive.current)onError(String(e));}
    finally{imageLock.current=false;if(imageAlive.current)setImageReading(false);}
  }

  const [manualSaving,setManualSaving]=useState(false);
  const [discardPrompt,setDiscardPrompt]=useState(false);
  const discardResolver=useRef<null|((allow:boolean)=>void)>(null);
  const latest=useRef({dirty:false});
  const saveInfo=useRef(async()=>false);
  useEffect(()=>{setNote(current=>current.dirty?current:{value:mod.description,dirty:false,error:''});},[mod.description]);
  const [renameId, setRenameId] = useState<string | null>(null); const [renameName, setRenameName] = useState(''); const [managed, setManaged] = useState(mod.managed); const [adding, setAdding] = useState(false); const [addIds, setAddIds] = useState<string[]>([]);
  latest.current={dirty:note.dirty||managed!==mod.managed};
  const saving=useRef(false);
  saveInfo.current=async()=>{
    if(saving.current)return false;
    saving.current=true;setManualSaving(true);
    try {
      const ok=await onSave(note.value,managed);
      if(ok){setNote(current=>({...current,dirty:false,error:''}));discardResolver.current?.(true);discardResolver.current=null;setDiscardPrompt(false);}
      else setNote(current=>({...current,error:'未能保存，请重试'}));
      return ok;
    } finally {saving.current=false;setManualSaving(false);}
  };
  const guard=useCallback(async()=>{
    if(saving.current)return false;
    if(!latest.current.dirty)return true;
    if(discardResolver.current)return false;
    setDiscardPrompt(true);return new Promise<boolean>(resolve=>{discardResolver.current=resolve;});
  },[]);
  const decide=(allow:boolean)=>{if(allow){setNote({value:mod.description,dirty:false,error:''});setManaged(mod.managed);}setDiscardPrompt(false);discardResolver.current?.(allow);discardResolver.current=null;};
  useEffect(()=>{registerCloseGuard(guard);return()=>{registerCloseGuard(null);discardResolver.current?.(false);};},[guard,registerCloseGuard]);
  const renameBox = useRef<HTMLDivElement>(null);
  useEffect(() => { if (renameId) { renameBox.current?.scrollIntoView({block:'center'}); renameBox.current?.querySelector('input')?.focus({preventScroll:true}); } }, [renameId]);
  const [reviewReplacement, setReviewReplacement] = useState(false); const [replacementConfirmed, setReplacementConfirmed] = useState(false);
  const candidates = mods.filter(m => m.id !== mod.id && !m.retired);
  async function pickCover() {
    try { const path = await open({title:'选择 MOD 封面',filters:[{name:'图片',extensions:['png','jpg','jpeg','webp']}]}); if(typeof path==='string'){const result=await request<{url:string}>('previewImage',{path});setCropSource(result.url);} } catch(e) { onError(String(e)); }
  }
  return <><Modal title="MOD 详情" onClose={onClose} beforeClose={guard} drawer><div className="detail-title"><button type="button" className="detail-cover-viewer" aria-label="查看封面原图" title={mod.imagePath?'查看原图':undefined} disabled={busy||imageReading||!mod.imagePath} onClick={()=>void viewOriginal()} onKeyDown={e=>{if((e.ctrlKey||e.metaKey)&&e.key.toLowerCase()==='v'){e.preventDefault();e.stopPropagation();void pasteCover();}}}><ModImage mod={mod}/>{imageReading&&<span className="image-source-loading"><LoaderCircle size={19} className="spin"/></span>}</button><div><h3>{modName(mod)}</h3><div className="detail-badges"><span><Folder size={13}/>{memberSummary(mod)}</span><span className={`state-label ${mod.state}`}>{!mod.managed ? "待确认" : states[mod.state]}</span>{mod.members.length > 1 && <span><Layers size={13}/>多文件 MOD</span>}</div></div></div>{mod.identityChanged && !mod.retired && <div className="replacement-notice"><p>检测到成员文件变化或缺失，请核对当前文件后再恢复管理。</p>{!reviewReplacement ? <button className="secondary" disabled={busy} onClick={() => setReviewReplacement(true)}>核对当前文件</button> : <div className="replacement-review"><p>请先核对下方全部成员路径和文件。确认后只更新管理器中的文件关联，保留名称和备注。</p><label className="manage-check"><input type="checkbox" checked={replacementConfirmed} onChange={e => setReplacementConfirmed(e.target.checked)}/>我已核对成员文件，并同意将当前文件重新关联到此记录（保留名称和备注）</label><div className="dialog-actions compact"><button className="secondary" disabled={busy} onClick={() => { setReviewReplacement(false); setReplacementConfirmed(false); }}>取消</button><button className="primary" disabled={busy || !replacementConfirmed} onClick={async () => { if (await onAcceptReplacement()) { setReviewReplacement(false); setReplacementConfirmed(false); } }}>确认关联</button></div></div>}</div>}<div className="cover-actions cover-controls"><div className="cover-control-buttons"><button type="button" className="secondary" disabled={busy||imageReading} onClick={() => void pickCover()}><Upload size={14}/>{mod.imagePath ? '更换封面' : '添加封面'}</button><button type="button" className="secondary" disabled={busy||imageReading} onClick={()=>void pasteCover()}><ClipboardPaste size={14}/>粘贴图片</button><button type="button" className="secondary" aria-label="移除 MOD 图片" disabled={busy||imageReading||!mod.imagePath} onClick={() => void onImage(null)}><Trash2 size={14}/>移除图片</button></div><small>PNG / JPG / WebP · 最大 20 MB</small></div><div className="detail-status"><span>当前文件状态</span><button className={`switch ${mod.state === 'enabled' ? 'on' : ''}`} role="switch" aria-checked={mod.state === 'enabled'} aria-label="详情 MOD 启停" aria-busy={toggling} disabled={busy || !mod.managed || mod.state === 'missing'} onClick={onToggle}><span>{toggling && <LoaderCircle size={12} className="spin"/>}</span></button><strong>{!mod.managed ? '待确认' : states[mod.state]}</strong></div>{mod.state === 'mixed' && <div className="mixed-files-note"><p>部分成员的启停状态不同，可能有新加入的内部文件尚未禁用。新文件保持原样，由你决定是否一起禁用。</p><div className="mixed-files-actions"><button className="secondary" disabled={busy || !mod.managed} onClick={() => onUnify(false)}>一起禁用</button><button className="secondary" disabled={busy || !mod.managed} onClick={() => onUnify(true)}>全部开启</button></div></div>}{!!mod.preservedDisabledFiles && <p className="preserved-files-note">{mod.preservedDisabledFiles} 个内部文件由外部禁用，启停时保持原样。</p>}<form id="mod-detail-form" onSubmit={e => { e.preventDefault(); void saveInfo.current(); }}><label className="field">描述与备注<textarea aria-label="描述与备注" disabled={busy||manualSaving} value={note.value} onChange={e => setNote({value:e.target.value,dirty:e.target.value!==mod.description,error:''})} placeholder="记下这个 MOD 的作用、使用建议或搭配…" rows={4}/></label><div className={`note-save-state ${note.error?'failed':''}`} role="status">{note.error?<><AlertCircle size={13}/><span>尚未保存，修改仍保留</span><button type="button" className="text-button" onClick={()=>void saveInfo.current()}>重试保存</button></>:manualSaving?<><LoaderCircle size={13} className="spin"/>保存中…</>:note.dirty?'未保存':<><Check size={13}/>备注已保存</>}</div><label className="manage-check"><input type="checkbox" checked={managed} disabled={busy||manualSaving||mod.retired} onChange={e => setManaged(e.target.checked)}/>{mod.retired ? '旧目录历史记录，仅保留名称和备注' : '确认这些文件是 MOD，纳入启停管理'}</label></form><div className="members-heading"><h3>关联文件 <span>{mod.members.length}</span></h3><button className="secondary" onClick={() => setAdding(!adding)} disabled={busy || mod.retired}><Plus size={15}/>添加成员</button></div><div className="members">{mod.members.map(member => <div className="member" key={member.id}>{member.kind === 'directory' ? <Folder size={18}/> : <File size={18}/>}<div><strong>{basename(member.originalPath)}</strong><span title={displayPath(member.currentPath)}>{displayPath(member.currentPath)}</span>{!member.exists && <small className="missing-text">文件缺失；历史名称和备注仍被保留</small>}{member.kind === 'directory' && !!member.leaves?.length && <details className="tree-inventory"><summary>内部文件 · {member.leaves.length} 个</summary><ul>{member.leaves.map(leaf => <li key={leaf.original} title={displayPath(leaf.current)} className={leaf.exists ? '' : 'missing-text'}><File size={12}/><span>{displayPath(leaf.current).slice(displayPath(member.currentPath).length + 1)}{!leaf.exists && ' · 缺失'}{leaf.disabledByManager === false && leaf.current !== leaf.original && ' · 外部禁用'}</span></li>)}</ul></details>}</div><button type="button" className="member-rename" title="修改真实文件或文件夹名称" disabled={busy || !member.exists || mod.retired || mod.state === 'error'} onClick={() => {setRenameId(member.id);setRenameName(basename(member.originalPath));}}>改名</button>{mod.members.length > 1 && <IconButton label={`移出 ${basename(member.originalPath)}`} disabled={busy} onClick={async() => {if(await guard())await onMembers(mod.members.filter(m => m.id !== member.id).map(m => m.id));}}><X size={15}/></IconButton>}</div>)}</div>{renameId && <div className="rename-editor" ref={renameBox}><label className="field">修改真实名称<input aria-label="真实文件名称" value={renameName} onChange={e => setRenameName(e.target.value)} maxLength={240}/><small>将修改磁盘上的文件或文件夹名称。文件需保留扩展名，禁用后缀由管理器处理。部分 MOD 依赖固定名称，改名前请确认兼容性。</small></label><div className="dialog-actions"><button className="secondary" disabled={busy} onClick={() => setRenameId(null)}>取消改名</button><button className="primary" disabled={busy || !renameName.trim()} onClick={async () => {if(await onRename(renameId,renameName))setRenameId(null);}}>确认改名</button></div></div>}{adding && <div className="add-members"><p>选取现有 MOD 的全部文件加入分组。文件不会被复制或重命名。</p>{candidates.length ? candidates.map(m => <label key={m.id}><input type="checkbox" checked={addIds.includes(m.id)} onChange={e => setAddIds(e.target.checked ? [...addIds, m.id] : addIds.filter(id => id !== m.id))}/>{modName(m)}<small>{m.members.length} 个文件</small></label>) : <p>暂无可加入的其他 MOD。</p>}<button className="secondary" disabled={busy || !addIds.length} onClick={async () => { if (await guard() && await onAdd(addIds)) { setAdding(false); setAddIds([]); } }}>加入分组</button></div>}{mod.members.length > 1 && <div className="ungroup-line"><span>解散后，每个成员独立显示。文件保持当前状态。</span><button className="text-danger" onClick={async() => {if(await guard())await onUngroup();}} disabled={busy}>解散分组</button></div>}<div className="drawer-actions"><button type="button" className="text-danger" disabled={busy} onClick={async()=>{if(await guard())onDelete();}}><Trash2 size={15}/>{mod.retired || mod.members.every(f => !f.exists) ? '移除记录' : '删除 MOD'}</button><button className="secondary" data-dismiss onClick={onClose}>关闭</button><button className="primary" form="mod-detail-form" type="submit" disabled={busy||manualSaving}>{manualSaving?<><LoaderCircle size={15} className="spin"/>保存中</>:!note.dirty&&managed===mod.managed?<><Check size={15}/>已保存</>:'保存信息'}</button></div></Modal>
  {originalView&&<Modal title="封面原图" wide onClose={()=>setOriginalView(null)} footer={<div className="dialog-actions"><button type="button" className="secondary" onClick={()=>setOriginalSize(value=>!value)}>{originalSize?<ZoomOut size={15}/>:<ZoomIn size={15}/>} {originalSize?'适应窗口':'原始尺寸'}</button><button className="secondary" data-dismiss>关闭</button></div>}><div className={`original-image-view ${originalSize?'actual-size':''}`}><img src={originalView} alt="MOD 封面原图"/></div></Modal>}
  {discardPrompt&&<Modal title="保存 MOD 信息？" onClose={()=>decide(false)} footer={<div className="dialog-actions"><button className="secondary" disabled={manualSaving} onClick={()=>decide(false)}>继续编辑</button><button className="secondary" disabled={manualSaving} onClick={()=>decide(true)}>放弃修改</button><button className="primary" disabled={manualSaving} onClick={()=>void saveInfo.current()}>{manualSaving?'保存中…':'保存并继续'}</button></div>}><p className="dialog-copy">备注或管理选项尚未保存，请选择如何处理。</p>{note.error&&<p role="alert" className="picture-error">{note.error}</p>}</Modal>}
  {cropSource&&<ImageCropper source={cropSource} onCancel={()=>setCropSource(null)} onError={onError} onConfirm={async dataUrl=>{const image=await request<{path:string;originalPath:string}>('saveCrop',{dataUrl,sourceUrl:cropSource});if(!await onImage(image.path,image.originalPath))throw new Error('封面尚未保存，请重试');setCropSource(null);}}/>}</>;
}
function GroupEditor({ busy, count, onClose, onSave }: { busy: boolean; count: number; onClose: () => void; onSave: (name: string, description: string) => Promise<void> }) {
  const [description, setDescription] = useState('');
  return <Modal title="创建逻辑 MOD" onClose={onClose}><p className="dialog-copy">将所选 {count} 个 MOD 的成员作为一个整体管理，只修改关联关系。</p><form onSubmit={e => { e.preventDefault(); void onSave('', description); }}><label className="field">备注<textarea rows={3} value={description} onChange={e => setDescription(e.target.value)} placeholder="关于这组 MOD…"/></label><div className="dialog-actions"><button type="button" className="secondary" data-dismiss onClick={onClose}>取消</button><button className="primary" disabled={busy}>创建分组</button></div></form></Modal>;
}



function DeleteModDialog({ mod, busy, onClose, onConfirm }: { mod: Mod; busy: boolean; onClose: () => void; onConfirm: (recordOnly: boolean) => Promise<void> }) {
  const recordOnly = !!mod.retired || mod.members.every(f => !f.exists);
  const existing = mod.members.filter(f => f.exists).length;
  return <Modal title={recordOnly ? '移除 MOD 记录？' : '将 MOD 移入回收站？'} onClose={onClose} footer={<div className="dialog-actions"><button className="secondary" disabled={busy} data-dismiss>取消</button><button className="danger" disabled={busy} onClick={() => void onConfirm(recordOnly)}>{busy && <LoaderCircle size={15} className="spin"/>}{recordOnly ? '移除记录' : '移入回收站'}</button></div>}>
    <p className="dialog-copy">{recordOnly ? `移除「${modName(mod)}」的管理信息、分组和备注。不会删除磁盘上的任何文件。` : `将「${modName(mod)}」的 ${existing} 个现有成员移入 Windows 回收站，并移除管理记录。文件夹成员包括其内部内容。`}</p>
    {!recordOnly && <p className="delete-recovery-note">删除后可点击“撤销删除”，或在设置的“最近删除”中恢复原文件、备注、封面与分组。请勿清空回收站。</p>}
    <div className="delete-member-list">{mod.members.map(f => <div key={f.id}>{f.kind === 'directory' ? <Folder size={17}/> : <File size={17}/>}<div><strong>{basename(f.originalPath)}</strong><small>{displayPath(f.currentPath)}</small></div>{!f.exists && <span>已缺失</span>}</div>)}</div>
    {!recordOnly && <small className="delete-limits">支持本地固定磁盘。回收站不可用时会报错；部分失败时保留剩余文件和记录。</small>}
  </Modal>;
}

function errorAdvice(error:string) {
  if (/正在运行|退出游戏/.test(error)) return '先退出对应游戏，再重试操作。';
  if (/同名|冲突|覆盖/.test(error)) return '打开文件位置，核对原始文件和禁用文件；处理同名冲突后重试。';
  if (/占用|权限|拒绝访问|Access is denied/.test(error)) return '退出游戏并关闭正在使用这些文件的程序，确认目录可写后重试。';
  if (/回滚|恢复未完成|状态不一致/.test(error)) return '核对每个成员的实际路径和状态，解决占用或冲突后按提示重试。';
  if (/身份|替换|关联|发生变化/.test(error)) return '在详情中核对全部成员，确认文件来源后再重新关联。';
  if (/回收站/.test(error)) return '检查 Windows 回收站是否仍有原文件；恢复记录会保留，不会覆盖现有文件。';
  if (/不存在|缺失|目录/.test(error)) return '检查磁盘连接和目录路径，恢复缺失位置后刷新。';
  return '核对操作对象后重试；详细结果可展开查看。';
}
