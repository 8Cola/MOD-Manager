// Edit literal UI text only; never identifiers, user records, or real paths.
const ts = require('typescript'), fs = require('node:fs'), path = require('node:path');
function wording(text) { return text.replace(/\bMod\b/g, 'MOD').replace(/MOD Manager/g, 'Mod Manager').replace(/MOD-Manager/g, 'Mod-Manager'); }
function visitDir(dir) {
  for (const name of fs.readdirSync(dir)) {
    const file=path.join(dir,name);
    if (fs.statSync(file).isDirectory()) { visitDir(file); continue; }
    if (!/\.tsx?$/.test(name)) continue;
    let source=fs.readFileSync(file,'utf8');
    const ast=ts.createSourceFile(file,source,ts.ScriptTarget.Latest,true,name.endsWith('tsx')?ts.ScriptKind.TSX:ts.ScriptKind.TS), edits=[];
    function visit(node) {
      if (ts.isStringLiteral(node) || ts.isJsxText(node) || ts.isNoSubstitutionTemplateLiteral(node) || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) {
        const start=node.getStart(ast), end=node.end, text=source.slice(start,end);
        // Import names and file paths are not labels.
        if (!text.includes('./') && !text.includes('/src/') && !text.includes('\\')) {
          const changed=wording(text); if(changed!==text) edits.push({start,end,text:changed});
        }
      }
      ts.forEachChild(node,visit);
    }
    visit(ast);
    for(const edit of edits.sort((a,b)=>b.start-a.start)) source=source.slice(0,edit.start)+edit.text+source.slice(edit.end);
    if(edits.length) fs.writeFileSync(file,source);
  }
}
visitDir('src');
