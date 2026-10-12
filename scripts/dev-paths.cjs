// Normal source checkouts use Cargo's default target directory. This workspace
// keeps its existing build artifacts in the ignored local environment folder.
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
exports.debugExeRelative = fs.existsSync(path.join(root, '_local/target/debug/mod-manager.exe'))
  ? '_local/target/debug/mod-manager.exe'
  : 'src-tauri/target/debug/mod-manager.exe';
