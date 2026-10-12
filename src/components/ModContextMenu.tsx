import { useEffect, useRef } from 'react';
import { Copy, FileSearch, Info, Trash2 } from 'lucide-react';
import { modName, type Mod } from '../types';

export function ModContextMenu({ mod, x, y, busy, onClose, onDetails, onReveal, onCopy, onDelete }: {
  mod: Mod; x: number; y: number; busy: boolean; onClose: () => void;
  onDetails: () => void; onReveal: () => void; onCopy: () => void; onDelete: () => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement;
    root.current?.querySelector<HTMLButtonElement>('button')?.focus({ preventScroll: true });
    const close = (event: Event) => { if (!root.current?.contains(event.target as Node)) onClose(); };
    const resize = () => onClose();
    document.addEventListener('pointerdown', close); window.addEventListener('resize', resize);
    document.addEventListener('scroll', close, true);
    return () => { document.removeEventListener('pointerdown', close); window.removeEventListener('resize', resize); document.removeEventListener('scroll', close, true); previous?.focus({preventScroll:true}); };
  }, [onClose]);
  const act = (fn: () => void) => { onClose(); fn(); };
  return <div className="mod-context-menu" role="menu" aria-label={`${modName(mod)} 操作`} ref={root}
    style={{ left: Math.max(12, Math.min(x, innerWidth - 230)), top: Math.max(40, Math.min(y, innerHeight - 200)) }}
    onKeyDown={event => {
      if (event.key === 'Escape' || event.key === 'Tab') { event.preventDefault(); event.stopPropagation(); onClose(); }
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault(); const buttons = Array.from(root.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') || []);
        const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
        buttons[(at + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length]?.focus();
      }
    }}>
    <button role="menuitem" onClick={() => act(onDetails)}><Info size={16}/>查看详情</button>
    <button role="menuitem" disabled={busy || mod.retired} onClick={() => act(onReveal)}><FileSearch size={16}/>定位文件</button>
    <button role="menuitem" onClick={() => act(onCopy)}><Copy size={16}/>复制路径</button>
    <div className="menu-separator"/>
    <button role="menuitem" className="menu-danger" disabled={busy} onClick={() => act(onDelete)}><Trash2 size={16}/>{mod.retired || mod.members.every(f=>!f.exists) ? '移除记录' : '删除 MOD'}</button>
  </div>;
}
