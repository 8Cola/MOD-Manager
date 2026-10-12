import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from 'react';
import { createPortal } from 'react-dom';
import { Crop, Minus, Plus, RotateCcw, X } from 'lucide-react';
import { boundPan, cropRectangle } from './image-crop-math';
import type { CropPan } from './image-crop-math';
import './image-cropper.css';

export type ImageCropperProps = {
  source: string;
  onConfirm: (dataUrl: string) => Promise<void> | void;
  onCancel: () => void;
  onError: (message: string) => void;
};

export function ImageCropper({ source, onConfirm, onCancel, onError }: ImageCropperProps) {
  const titleId = useId();
  const hintId = useId();
  const zoomId = useId();
  const dialog = useRef<HTMLDivElement>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const drag = useRef<{ id: number; x: number; y: number; pan: CropPan; edge: number } | null>(null);
  const callbacks = useRef({ onCancel, onError });
  callbacks.current = { onCancel, onError };
  const busyRef = useRef(false);
  const [dimensions, setDimensions] = useState<{ width: number; height: number } | null>(null);
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState<CropPan>({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const geometry = useRef({ dimensions, zoom, pan });
  geometry.current = { dimensions, zoom, pan };

  useEffect(() => {
    let active = true;
    setDimensions(null);
    setError('');
    setZoom(1);
    setPan({ x: 0, y: 0 });
    const image = new Image();
    imageRef.current = null;
    image.onload = () => {
      if (!active) return;
      if (!image.naturalWidth || !image.naturalHeight) return image.onerror?.(new Event('error'));
      imageRef.current = image;
      setDimensions({ width: image.naturalWidth, height: image.naturalHeight });
    };
    image.onerror = () => {
      if (!active) return;
      const message = '无法读取这张图片，请重新选择。';
      setError(message);
      callbacks.current.onError(message);
    };
    image.src = source;
    return () => { active = false; image.onload = null; image.onerror = null; };
  }, [source]);

  useEffect(() => {
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    dialog.current?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (!busyRef.current) callbacks.current.onCancel();
      }
      if (event.key !== 'Tab' || !dialog.current) return;
      const items = Array.from(dialog.current.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex="0"]'));
      const first = items[0];
      const last = items[items.length - 1];
      if (!first) { event.preventDefault(); dialog.current.focus(); return; }
      const focused = document.activeElement;
      if (event.shiftKey && (focused === first || !items.includes(focused as HTMLElement))) {
        event.preventDefault(); last.focus();
      } else if (!event.shiftKey && (focused === last || !items.includes(focused as HTMLElement))) {
        event.preventDefault(); first.focus();
      }
    };
    const focusin = (event: FocusEvent) => {
      if (dialog.current && !dialog.current.contains(event.target as Node)) dialog.current.focus();
    };
    // Window capture intercepts Escape before the underlying dialog's document listener.
    window.addEventListener('keydown', keydown, true);
    document.addEventListener('focusin', focusin);
    return () => {
      window.removeEventListener('keydown', keydown, true);
      document.removeEventListener('focusin', focusin);
      document.body.style.overflow = previousOverflow;
      if (previousFocus?.isConnected) previousFocus.focus();
    };
  }, []);

  function changeZoom(next: number) {
    const { dimensions: size, zoom: oldZoom, pan: oldPan } = geometry.current;
    if (!size || busyRef.current) return;
    const value = Math.max(1, Math.min(4, next));
    const nextPan = boundPan(size.width, size.height, value, { x: oldPan.x * value / oldZoom, y: oldPan.y * value / oldZoom });
    geometry.current = { dimensions: size, zoom: value, pan: nextPan };
    setZoom(value);
    setPan(nextPan);
    drag.current = null;
    setDragging(false);
  }

  useEffect(() => {
    const element = viewport.current;
    if (!element) return;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      event.stopPropagation();
      changeZoom(geometry.current.zoom * Math.exp(-event.deltaY * (event.deltaMode === 1 ? 0.04 : 0.002)));
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, []);

  function movePan(next: CropPan) {
    const { dimensions: size, zoom: currentZoom } = geometry.current;
    if (!size || busyRef.current) return;
    const nextPan = boundPan(size.width, size.height, currentZoom, next);
    geometry.current.pan = nextPan;
    setPan(nextPan);
  }
  function pointerDown(event: ReactPointerEvent<HTMLDivElement>) {
    if (!dimensions || busyRef.current || event.button !== 0 || drag.current) return;
    event.preventDefault();
    event.currentTarget.focus();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { id: event.pointerId, x: event.clientX, y: event.clientY, pan: geometry.current.pan, edge: event.currentTarget.getBoundingClientRect().width };
    setDragging(true);
  }
  function pointerMove(event: ReactPointerEvent<HTMLDivElement>) {
    const start = drag.current;
    if (!start || start.id !== event.pointerId) return;
    movePan({ x: start.pan.x + (event.clientX - start.x) / start.edge, y: start.pan.y + (event.clientY - start.y) / start.edge });
  }
  function pointerEnd(event: ReactPointerEvent<HTMLDivElement>) {
    if (drag.current?.id !== event.pointerId) return;
    drag.current = null;
    setDragging(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  }
  function keyboardPan(event: ReactKeyboardEvent<HTMLDivElement>) {
    const step = event.shiftKey ? 0.08 : 0.02;
    const offsets: Record<string, CropPan> = { ArrowLeft: { x: -step, y: 0 }, ArrowRight: { x: step, y: 0 }, ArrowUp: { x: 0, y: -step }, ArrowDown: { x: 0, y: step } };
    if (offsets[event.key]) {
      event.preventDefault(); event.stopPropagation();
      movePan({ x: geometry.current.pan.x + offsets[event.key].x, y: geometry.current.pan.y + offsets[event.key].y });
    } else if (event.key === '+' || event.key === '=' || event.key === '-') {
      event.preventDefault(); changeZoom(geometry.current.zoom + (event.key === '-' ? -0.1 : 0.1));
    }
  }
  function reset() {
    geometry.current = { dimensions, zoom: 1, pan: { x: 0, y: 0 } };
    setZoom(1); setPan({ x: 0, y: 0 }); drag.current = null; setDragging(false);
  }
  async function confirm() {
    const image = imageRef.current;
    const { dimensions: size, zoom: currentZoom, pan: currentPan } = geometry.current;
    if (!image || !size || busyRef.current) return;
    busyRef.current = true;
    setBusy(true); setError('');
    try {
      const crop = cropRectangle(size.width, size.height, currentZoom, currentPan);
      const canvas = document.createElement('canvas');
      canvas.width = 512; canvas.height = 512;
      const context = canvas.getContext('2d');
      if (!context) throw new Error('无法创建图片，请重试。');
      context.imageSmoothingEnabled = true;
      context.imageSmoothingQuality = 'high';
      context.drawImage(image, crop.x, crop.y, crop.size, crop.size, 0, 0, 512, 512);
      await onConfirm(canvas.toDataURL('image/png'));
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(message); callbacks.current.onError(message);
    } finally { busyRef.current = false; setBusy(false); }
  }

  const shortest = dimensions ? Math.min(dimensions.width, dimensions.height) : 1;
  return createPortal(<div className="image-cropper-overlay" onClick={event => event.stopPropagation()}>
    <div className="image-cropper-dialog" ref={dialog} role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={hintId} aria-busy={busy} tabIndex={-1}>
      <header className="image-cropper-heading"><div><span className="image-cropper-eyebrow"><Crop size={14}/>图片裁剪</span><h2 id={titleId}>选好画面，再保存</h2></div><button type="button" className="icon-button" aria-label="取消裁剪" disabled={busy} onClick={onCancel}><X size={20}/></button></header>
      <p id={hintId} className="image-cropper-copy">拖动图片调整位置，滚轮或滑块缩放。保存为 512 × 512 方形图片。</p>
      <div className="image-cropper-stage"><div ref={viewport} className={`image-cropper-viewport${dragging ? ' is-dragging' : ''}`} tabIndex={0} role="group" aria-label="裁剪预览：方向键移动图片，加减键缩放" aria-describedby={hintId} onPointerDown={pointerDown} onPointerMove={pointerMove} onPointerUp={pointerEnd} onPointerCancel={pointerEnd} onLostPointerCapture={pointerEnd} onKeyDown={keyboardPan}>
        {dimensions ? <><img src={source} alt="" draggable={false} style={{ width: `${dimensions.width / shortest * zoom * 100}%`, height: `${dimensions.height / shortest * zoom * 100}%`, left: `${50 + pan.x * 100}%`, top: `${50 + pan.y * 100}%` }}/><div className="image-cropper-grid" aria-hidden="true"/><span className="image-cropper-corner top-left" aria-hidden="true"/><span className="image-cropper-corner top-right" aria-hidden="true"/><span className="image-cropper-corner bottom-left" aria-hidden="true"/><span className="image-cropper-corner bottom-right" aria-hidden="true"/></> : <span className="image-cropper-loading">{error ? '图片读取失败' : '正在读取图片…'}</span>}
      </div><span className="image-cropper-size" aria-hidden="true">1 : 1</span></div>
      <div className="image-cropper-zoom-heading"><label htmlFor={zoomId}>缩放</label><output htmlFor={zoomId}>{Math.round(zoom * 100)}%</output></div>
      <div className="image-cropper-zoom"><button type="button" className="icon-button" aria-label="缩小图片" disabled={!dimensions || busy || zoom <= 1} onClick={() => changeZoom(zoom - 0.1)}><Minus size={17}/></button><input id={zoomId} type="range" min="1" max="4" step="0.01" value={zoom} disabled={!dimensions || busy} onChange={event => changeZoom(Number(event.target.value))}/><button type="button" className="icon-button" aria-label="放大图片" disabled={!dimensions || busy || zoom >= 4} onClick={() => changeZoom(zoom + 0.1)}><Plus size={17}/></button></div>
      <div className="image-cropper-assist"><span>也可用方向键调整位置</span><button type="button" disabled={!dimensions || busy} onClick={reset}><RotateCcw size={13}/>重置</button></div>
      {error && <p className="image-cropper-error" role="alert">{error}</p>}
      <footer className="image-cropper-actions"><button type="button" className="secondary" disabled={busy} onClick={onCancel}>取消</button><button type="button" className="primary" disabled={!dimensions || busy} onClick={() => void confirm()}>{busy ? '正在保存…' : '确认裁剪'}</button></footer>
    </div>
  </div>, document.body);
}
