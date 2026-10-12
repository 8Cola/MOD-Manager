import { FolderOpen, Plus, X } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import type { ModFolder } from '../types';

export function FolderEditor({ folders, busy, onChange, onError }: {
  folders: ModFolder[]; busy: boolean; onChange: (folders: ModFolder[]) => void; onError: (message: string) => void;
}) {
  function update(index: number, change: Partial<ModFolder>) {
    onChange(folders.map((folder, i) => i === index ? { ...folder, ...change } : folder));
  }
  async function browse(index: number) {
    try {
      const path = await open({ title: `选择${folders[index].name || ' MOD '}目录`, directory: true });
      if (typeof path === 'string') update(index, { path });
    } catch (error) { onError(String(error)); }
  }
  return <section className="folder-editor">
    <div className="folder-editor-heading"><strong>MOD 目录</strong><button type="button" className="secondary" disabled={busy || folders.length >= 32} onClick={() => onChange([...folders, { id: '', name: '', path: '' }])}><Plus size={15}/>添加目录</button></div>
    <p>按用途分别添加，例如：常规 MOD、玩法 MOD、光影或材质包。</p>
    <div className="folder-editor-list">{folders.map((folder, index) => <div className="folder-editor-row" key={folder.id || `new-${index}`}>
      <label className="folder-name-field"><input required maxLength={80} disabled={busy} aria-label={`目录 ${index + 1} 备注名`} value={folder.name} placeholder="备注名，如：光影" onChange={e => update(index, { name: e.target.value })}/></label>
      <label className="folder-path-field"><div className="path-field"><input required disabled={busy} aria-label={`目录 ${index + 1} 路径`} value={folder.path} placeholder="选择该类型的加载目录" onChange={e => update(index, { path: e.target.value })}/><button type="button" className="icon-button" disabled={busy} aria-label={`选择目录 ${index + 1}`} onClick={() => void browse(index)}><FolderOpen size={18}/></button></div></label>
      <button type="button" className="icon-button folder-remove" disabled={busy || folders.length === 1} title="移除目录配置，不删除文件" aria-label={`移除目录 ${index + 1}`} onClick={() => onChange(folders.filter((_, i) => i !== index))}><X size={17}/></button>
    </div>)}</div>
    <small>目录必须互相独立。移除目录只修改配置，禁用中的 MOD 需先恢复。</small>
  </section>;
}
