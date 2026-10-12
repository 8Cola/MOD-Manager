import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { ClipboardPaste, ImagePlus, LoaderCircle, Trash2 } from 'lucide-react';
import { request } from '../api';

export function ImageSourcePicker({ label, className = '', disabled, children, onFile, onImage, onRemove }: {
  label: string; className?: string; disabled?: boolean; children: ReactNode;
  onFile: () => void | Promise<void>; onImage: (url: string) => void; onRemove?: () => void | Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [reading, setReading] = useState(false);
  const [error, setError] = useState('');
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const locked = useRef(false);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) setOpen(false); };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open]);
  async function paste() {
    if (disabled || locked.current) return;
    locked.current = true; setReading(true); setError('');
    try {
      const result = await request<{ url: string }>('clipboardImage');
      if (alive.current) { setOpen(false); trigger.current?.focus(); onImage(result.url); }
    } catch (e) { if (alive.current) { setOpen(true); setError(String(e)); } }
    finally { locked.current = false; if (alive.current) setReading(false); }
  }
  return <div className={`image-source-picker ${className}`} ref={root} onKeyDown={event => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'v') { event.preventDefault(); event.stopPropagation(); void paste(); }
    if (event.key === 'Escape' && open) { event.preventDefault(); event.stopPropagation(); setOpen(false); trigger.current?.focus(); }
    if (event.key === 'ArrowDown' && event.target === trigger.current) { event.preventDefault(); setOpen(true); requestAnimationFrame(() => root.current?.querySelector<HTMLButtonElement>('[role=menuitem]')?.focus()); }
  }}>
    <button type="button" ref={trigger} className="image-source-trigger" aria-label={label} aria-haspopup="menu" aria-expanded={open} disabled={disabled || reading} title="选择图片来源" onClick={() => { setError(''); setOpen(v => !v); }}>{children}{reading && <span className="image-source-loading"><LoaderCircle size={20} className="spin"/></span>}</button>
    {open && <div className="image-source-menu" role="menu" aria-label="图片来源" onKeyDown={event => {
      if (!['ArrowDown','ArrowUp','Home','End'].includes(event.key)) return;
      event.preventDefault(); event.stopPropagation();
      const items = [...(root.current?.querySelectorAll<HTMLButtonElement>('[role=menuitem]') || [])];
      const index = items.indexOf(document.activeElement as HTMLButtonElement);
      items[event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (index + (event.key === 'ArrowUp' ? -1 : 1) + items.length) % items.length]?.focus();
    }}>
      <button type="button" role="menuitem" disabled={reading} onClick={() => void paste()}><ClipboardPaste size={16}/><span>{reading ? '读取图片…' : '粘贴图片'}</span></button>
      <button type="button" role="menuitem" disabled={reading} onClick={() => { setOpen(false); trigger.current?.focus(); void onFile(); }}><ImagePlus size={16}/>选择文件</button>
      {onRemove && <button type="button" role="menuitem" disabled={reading} onClick={() => { setOpen(false); trigger.current?.focus(); void onRemove(); }}><Trash2 size={16}/>移除图片</button>}
      {error && <p role="alert">{error}</p>}
    </div>}
  </div>;
}
