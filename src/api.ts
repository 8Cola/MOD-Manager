import { invoke, convertFileSrc, isTauri } from '@tauri-apps/api/core';
import type { Snapshot } from './types';
export const desktop = isTauri();
export const request = <T = Snapshot>(action: string, args: Record<string, unknown> = {}) => invoke<T>('request', { request: { action, ...args } });
export const imageUrl = (path: string | null) => path && desktop ? convertFileSrc(path) : undefined;
