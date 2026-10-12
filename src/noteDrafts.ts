export type NoteState = { value:string; dirty:boolean; saving:boolean; error:string };
type Draft = { value:string; revision:number; saved:number; saving:boolean; error:string; timer:ReturnType<typeof setTimeout>|null; pending:Promise<void>|null };
/** A single queue per Mod: a response only acknowledges the exact revision written. */
export class NoteDrafts {
  private entries=new Map<string,Draft>();
  private listeners=new Map<string,Set<()=>void>>();
  constructor(private write:(id:string,description:string)=>Promise<void>,private failure:(message:string)=>void){}
  read(id:string,initial=''):NoteState {
    if(!this.entries.has(id))this.entries.set(id,{value:initial,revision:0,saved:0,saving:false,error:'',timer:null,pending:null});
    const d=this.entries.get(id)!;return {value:d.value,dirty:d.revision!==d.saved,saving:d.saving,error:d.error};
  }
  subscribe(id:string,fn:()=>void){let listeners=this.listeners.get(id);if(!listeners){listeners=new Set();this.listeners.set(id,listeners);}listeners.add(fn);return ()=>{listeners.delete(fn);};}
  refresh(id:string,value:string){const d=this.entries.get(id);if(!d){this.read(id,value);return;}if(!d.saving&&d.revision===d.saved&&d.value!==value){d.value=value;this.changed(id);}}
  private changed(id:string){this.listeners.get(id)?.forEach(fn=>fn());}
  update(id:string,value:string){this.read(id);const d=this.entries.get(id)!;if(d.value===value)return;d.value=value;d.revision++;d.error='';if(d.timer)clearTimeout(d.timer);d.timer=setTimeout(()=>{void this.flush(id).catch(()=>{});},500);this.changed(id);}
  async flush(id:string):Promise<void>{
    const d=this.entries.get(id);if(!d)return;
    if(d.timer){clearTimeout(d.timer);d.timer=null;}
    if(d.pending){await d.pending;if(d.revision!==d.saved)return this.flush(id);return;}
    if(d.revision===d.saved)return;
    const revision=d.revision,description=d.value;d.saving=true;d.error='';this.changed(id);
    d.pending=this.write(id,description).then(()=>{d.saved=revision;}).catch(e=>{d.error=String(e);this.failure(`备注未能保存：${String(e)}`);throw e;}).finally(()=>{d.saving=false;d.pending=null;this.changed(id);});
    await d.pending;
    if(d.revision!==d.saved)return this.flush(id);
  }
  async flushAll(){for(const id of this.entries.keys())await this.flush(id);}
  retain(ids:Set<string>){for(const [id,d] of this.entries)if(!ids.has(id)&&!d.pending&&d.revision===d.saved)this.clear(id);}
  clear(id:string){const d=this.entries.get(id);if(d?.timer)clearTimeout(d.timer);this.entries.delete(id);}
}
