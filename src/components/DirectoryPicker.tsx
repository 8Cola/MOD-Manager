import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { Check, ChevronDown, Folder, History } from 'lucide-react';
import { displayPath, type ModFolder } from '../types';

export function DirectoryPicker({ folders, value, hasHistory, disabled, onChange }: {
  folders: ModFolder[]; value: string; hasHistory: boolean; disabled: boolean; onChange: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const listId = useId();
  const items = [...folders.map(f => ({ id: f.id, name: f.name, detail: displayPath(f.path) })),
    ...(hasHistory ? [{ id: '__history', name: '历史记录 · 仅名称与备注', detail: '已移除目录中保留的管理信息' }] : [])];
  const current = items.find(item => item.id === value) || items[0];
  useEffect(() => {
    if (!open) return;
    const options = root.current?.querySelectorAll<HTMLButtonElement>('[role="option"]');
    options?.[Math.max(0, items.findIndex(item => item.id === value))]?.focus();
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open, value]);
  useEffect(() => { if (disabled) setOpen(false); }, [disabled]);
  const close = () => { setOpen(false); trigger.current?.focus(); };
  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape' && open) { event.preventDefault(); event.stopPropagation(); close(); }
    const options = Array.from(root.current?.querySelectorAll<HTMLButtonElement>('[role="option"]') || []);
    const index = options.indexOf(document.activeElement as HTMLButtonElement);
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault();
      if (!open) { setOpen(true); return; }
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1
        : (index + (event.key === 'ArrowDown' ? 1 : -1) + options.length) % options.length;
      options[next]?.focus();
    }
  };
  return <div className="directory-picker" ref={root} onKeyDown={keyDown}
    onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node)) setOpen(false); }}>
    <button className={`directory-trigger ${open ? 'expanded' : ''}`} ref={trigger} type="button"
      aria-label="当前 MOD 目录" aria-haspopup="listbox" aria-expanded={open} aria-controls={listId}
      disabled={disabled} onClick={() => setOpen(!open)}>
      {value === '__history' ? <History size={17}/> : <Folder size={17}/>}
      <span title={current?.name}>{current?.name}</span><ChevronDown size={16}/>
    </button>
    {open && <div className="directory-menu" id={listId} role="listbox" aria-label="选择管理目录">
      <div className="directory-menu-caption">管理目录</div>
      <div className="directory-options">{items.map(item => <button type="button" role="option"
        aria-label={item.name} aria-selected={item.id === value} tabIndex={item.id === value ? 0 : -1}
        className={`directory-option ${item.id === '__history' ? 'directory-history' : ''}`} key={item.id}
        onClick={() => { onChange(item.id); close(); }}>
        {item.id === '__history' ? <History size={17}/> : <Folder size={17}/>}
        <span><strong>{item.name}</strong><small title={item.detail}>{item.detail}</small></span>
        {item.id === value && <Check size={16} className="directory-check"/>}
      </button>)}</div>
    </div>}
  </div>;
}
