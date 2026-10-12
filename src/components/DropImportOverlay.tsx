import { useEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { FolderOpen, Upload, LoaderCircle, CircleHelp } from 'lucide-react';
import { desktop } from '../api';
import { displayPath } from '../types';

export type DropTarget = { gameId: string; folderId: string; gameName: string; folderName: string; path: string };
export type ImportProgress = {operationId:string;phase:string;copiedBytes:number;totalBytes:number;currentName:string;index:number;count:number};
const bytes=(n:number)=>n>=1073741824?`${(n/1073741824).toFixed(2)} GB`:n>=1048576?`${(n/1048576).toFixed(1)} MB`:n>=1024?`${(n/1024).toFixed(1)} KB`:`${n} B`;
export function DropImportOverlay({ target, blocked, progress, cancelRequested, onCancel, onImport, onError }: {
  progress:ImportProgress|null;cancelRequested:boolean;onCancel:()=>void;
  target: DropTarget | null; blocked: string; onImport: (paths: string[], target: DropTarget) => Promise<void>; onError:(error:string)=>void;
}) {
  const latest = useRef({ target, blocked, onImport, onError }); latest.current = { target, blocked, onImport, onError };
  const captured = useRef<DropTarget | null>(null);
  const working = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [view, setView] = useState<{target:DropTarget|null; reason:string; count:number; copying:boolean; closing:boolean}|null>(null);
  useEffect(()=>{
    const destination=captured.current;
    if(!working.current && destination && (blocked || target?.gameId!==destination.gameId || target?.folderId!==destination.folderId || target?.path!==destination.path)) {
      captured.current=null;
      setView(v=>v?{...v,target:null,reason:'页面已变化，请重新拖入文件'}:null);
    }
  },[blocked,target?.gameId,target?.folderId,target?.path]);
  useEffect(() => {
    if (!desktop) return;
    let disposed = false; let off: (()=>void) | undefined;
    function close() {
      setView(v=>v ? {...v,closing:true}:null);
      if(timer.current) clearTimeout(timer.current);
      timer.current=setTimeout(()=>{if(!disposed)setView(null);},180);
    }
    void getCurrentWindow().onDragDropEvent(({payload}) => {
      if(disposed || working.current) return;
      if(payload.type==='enter') {
        if(timer.current) clearTimeout(timer.current);
        captured.current=latest.current.blocked ? null : latest.current.target;
        setView({target:captured.current,reason:latest.current.blocked,count:payload.paths.length,copying:false,closing:false});
      } else if(payload.type==='leave') {captured.current=null;close();}
      else if(payload.type==='drop') {
        const destination=captured.current;captured.current=null;
        // Recheck current page: navigating or opening a dialog never redirects an in-flight drop.
        if(!destination || latest.current.blocked || latest.current.target?.gameId!==destination.gameId || latest.current.target?.folderId!==destination.folderId || latest.current.target?.path!==destination.path) {close();return;}
        working.current=true;
        setView({target:destination,reason:'',count:payload.paths.length,copying:true,closing:false});
        void latest.current.onImport(payload.paths,destination).finally(()=>{working.current=false;if(!disposed)close();});
      }
    }).then(fn=>{if(disposed)fn();else off=fn;}).catch(e=>{if(!disposed)latest.current.onError(`拖放功能未能启动，请重启程序重试：${String(e)}`);});
    return ()=>{disposed=true;off?.();if(timer.current)clearTimeout(timer.current);};
  },[]);
  if(!view) return null;
  return <div className={`drop-import-overlay ${view.closing?'closing':''} ${!view.target?'blocked':''}`} role="status" aria-live="polite">
    <div className="drop-import-panel"><span className="drop-import-symbol">{view.copying?<LoaderCircle className="spin" size={30}/>:view.target?<Upload size={30}/>:<CircleHelp size={30}/>}</span>
      <h2>{view.copying?'正在导入':view.target?'松手，添加到当前目录':'暂时不能导入'}</h2>
      {view.target?<><div className="drop-import-destination"><FolderOpen size={17}/><strong>{view.target.gameName}</strong><span>·</span><span>{view.target.folderName}</span></div><p className="drop-import-path" title={displayPath(view.target.path)}>{displayPath(view.target.path)}</p>{view.copying?<div className="import-copy-progress"><small>{cancelRequested?'正在安全停止…':progress?.phase==='preparing'?'正在核对文件…':`项目 ${progress?.index||1} / ${view.count}`}</small>{progress?.currentName&&<span className="import-current-name" title={progress.currentName}>{progress.currentName}</span>}<progress aria-label="复制进度" value={progress?.copiedBytes||0} max={progress?.totalBytes||1}/><small>{bytes(progress?.copiedBytes||0)} / {bytes(progress?.totalBytes||0)}</small><button className="secondary" disabled={cancelRequested} onClick={onCancel}>{cancelRequested?'正在取消':'取消导入'}</button></div>:<small>{view.count} 个项目 · 保留原文件 · 同名不覆盖</small>}</>:<p>{view.reason||'请先添加游戏并选择 MOD 目录'}</p>}
    </div>
  </div>;
}
