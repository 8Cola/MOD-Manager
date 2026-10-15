import { memo, useEffect, useState } from 'react';
import { File, Folder, Layers, LoaderCircle, MoreHorizontal } from 'lucide-react';
import { imageUrl } from '../api';
import { memberSummary, modName, type Mod } from '../types';

const states={enabled:'已开启',disabled:'已关闭',mixed:'状态不一致',missing:'文件缺失',error:'需要检查'};

export function ModImage({mod}:{mod:Mod}) {
  const [failed,setFailed]=useState(false);
  useEffect(()=>setFailed(false),[mod.imagePath]);
  return <span className={`mod-symbol ${mod.members.length>1?'group':''}`}>{mod.imagePath&&!failed?<img src={imageUrl(mod.imagePath)} alt="" loading="lazy" decoding="async" onError={()=>setFailed(true)}/>:mod.members.some(m=>m.kind==='directory')?<Folder size={27}/>:mod.members.length>1?<Layers size={27}/>:<File size={27}/>}</span>;
}

interface Props {
  mod:Mod; selected:boolean; focused:boolean; selecting:boolean; busy:boolean; toggling:boolean;
  onSelect:(id:string,checked:boolean)=>void;
  onOpen:(id:string)=>void;
  onToggle:(id:string,enabled:boolean)=>void;
  onContext:(mod:Mod,x:number,y:number)=>void;
}

// Stable callbacks and unchanged MOD objects let search input, theme changes,
// and a different row's progress reuse rows instead of rendering every cover.
export const ModListRow=memo(function ModListRow({mod,selected,focused,selecting,busy,toggling,onSelect,onOpen,onToggle,onContext}:Props){
  const name=modName(mod);
  return <div className={`mod-row ${selected||focused?'selected':''}`} onContextMenu={e=>{e.preventDefault();onContext(mod,e.clientX,e.clientY);}} onKeyDown={e=>{if(e.key==='ContextMenu'||(e.shiftKey&&e.key==='F10')){e.preventDefault();const r=e.currentTarget.getBoundingClientRect();onContext(mod,r.x+100,r.y+30);}}}>
    {selecting&&<input className="row-check" type="checkbox" aria-label={`选择 ${name}`} checked={selected} onChange={e=>onSelect(mod.id,e.target.checked)}/>}
    <button className="mod-open" onClick={()=>onOpen(mod.id)}><ModImage mod={mod}/><span className="mod-copy"><strong><span className="mod-name-text">{name}</span></strong>{mod.description.trim()&&<span className="mod-description">{mod.description}</span>}{(mod.members.length>1||mod.members[0]?.kind==='directory')&&<span className="mod-metadata"><span>{mod.members.length>1?<Layers size={13}/>:<Folder size={13}/>} {memberSummary(mod)}</span></span>}</span></button>
    <div className="row-end"><span className={`state-label ${mod.state}`}><span/>{!mod.managed?'待确认':states[mod.state]}</span><button className={`switch ${mod.state==='enabled'?'on':''}`} role="switch" aria-checked={mod.state==='enabled'} aria-busy={toggling} aria-label={`${name} ${mod.state==='enabled'?'关闭':'开启'}`} onClick={()=>onToggle(mod.id,mod.state!=='enabled')} disabled={busy||!mod.managed||mod.state==='missing'}><span>{toggling&&<LoaderCircle size={12} className="spin"/>}</span></button><button className="icon-button" title={`查看 ${name} 详情`} aria-label={`查看 ${name} 详情`} onClick={()=>onOpen(mod.id)}><MoreHorizontal size={19}/></button></div>
  </div>;
});
