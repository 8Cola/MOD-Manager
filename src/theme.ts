import type { Theme } from './types';

declare global { interface Window { __MM_THEME__?: Theme } }

export function startupTheme(): Theme {
  const theme=window.__MM_THEME__;
  return theme==='dark'||theme==='light' ? theme : 'system';
}

export function applyTheme(preference: Theme): boolean {
  const dark=preference==='dark'||(preference==='system'&&matchMedia('(prefers-color-scheme: dark)').matches);
  const root=document.documentElement;
  root.dataset.theme=dark?'dark':'light';
  root.style.colorScheme=dark?'dark':'light';
  root.style.backgroundColor=dark?'#171d28':'#fcfdff';
  return dark;
}
