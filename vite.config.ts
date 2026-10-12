import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
export default defineConfig({plugins:[react(),tailwindcss()],clearScreen:false,server:{port:1420,strictPort:true,host:'127.0.0.1',watch:{ignored:['**/_local/**','**/.tools/**','**/src-tauri/**','**/output/**','**/release/**']}},envPrefix:['VITE_','TAURI_ENV_'],build:{target:'es2022'}});
