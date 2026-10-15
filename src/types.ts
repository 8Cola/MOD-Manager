export type Strategy = 'suffix' | 'move';
export type Theme = 'system' | 'light' | 'dark';
export interface ModFolder { id: string; name: string; path: string }
export interface Game { id: string; name: string; modDir: string; folders: ModFolder[]; imagePath: string | null; exePath: string | null; strategy: Strategy }
export interface LeafFile {original:string;current:string;exists:boolean;disabledByManager?:boolean|null}
export interface Member { leaves?: LeafFile[]; id: string; originalPath: string; currentPath: string; kind: 'file' | 'directory'; exists: boolean }
export interface Mod { id: string; gameId: string; folderId: string; name: string; description: string; imagePath?: string | null; originalImagePath?: string | null; managed: boolean; retired?: boolean; identityChanged?: boolean; preservedDisabledFiles?: number; state: 'enabled' | 'disabled' | 'mixed' | 'missing' | 'error'; members: Member[] }
export interface PendingImport {id:string;source:string;target:string;stage:string;status:string;detail:string;targetExists:boolean;stageExists:boolean;cleanupAvailable:boolean}
export interface Snapshot { revision?:number; pendingImports?:PendingImport[]; view?:ViewPreferences; recentDeletions?:RecentDeletion[]; games: Game[]; mods: Mod[]; settings: { theme: Theme; defaultStrategy: Strategy }; dataDir: string; monitoring: string; warnings: string[] }
export const basename = (path: string) => path.split(/[\\/]/).pop() || path;
export const displayPath = (path: string) => path.toUpperCase().startsWith('\\\\?\\UNC\\') ? '\\\\' + path.slice(8) : path.startsWith('\\\\?\\') ? path.slice(4) : path;
export const modName = (mod: Mod) => basename(mod.members[0]?.currentPath || mod.members[0]?.originalPath || '未命名 MOD');
export const memberSummary = (mod: Mod) => mod.members.length > 1 ? `${mod.members.length} 个成员` : mod.members[0]?.kind === 'directory' ? '文件夹' : '文件';
export interface ViewPreferences { gameId?: string; folderChoices?: Record<string,string>; scrolls?: Record<string,number> }
export interface RecentDeletion { id:string; name:string; status:string; at:string; gameId:string; folderId:string }
