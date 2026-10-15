# 开源目录说明

2026-10-12 已将本机开发环境及生成文件集中到根目录 `_local`。这些内容没有删除，但不应随源码上传；`.gitignore` 和源码打包脚本都已排除整个 `_local`。

保留并应上传的项目文件：

- `src/`：React 界面、样式、组件和应用资源。
- `src-tauri/src/`、`src-tauri/tests/`：Rust 实现及测试、必要测试图片。
- `src-tauri/icons/`、`capabilities/`、Tauri 配置和构建文件。
- `scripts/`：项目构建、打包及验收脚本，属于项目的一部分。
- `docs/`、`README.md`：项目说明和验证记录；历史记录中的旧目录路径保留为当时的记录。
- `Cargo.lock`、`package-lock.json`、`package.json`、工具链与前端配置：用于复现构建，应保留。

`_local` 内的分类：

| 文件夹 | 内容 |
| --- | --- |
| `tools/` | 本机 Rust/MSVC 工具链、下载缓存、旧的一次性开发脚本与临时数据 |
| `node_modules/` | 已下载的前端依赖，可由 `npm ci` 重新安装 |
| `target/` | Rust 编译缓存及 Debug/Release 构建结果 |
| `dist/` | 已生成的前端产物 |
| `output/`、`playwright-cli/` | 本机验收报告、截图和自动化临时记录 |
| `tauri-gen/` | Tauri 自动生成的结构定义 |
| `tsconfig.tsbuildinfo` | TypeScript 增量构建缓存 |
| `moved-files.json` | 本次整理的原位置与新位置对照 |

`release/` 是软件成品和源码压缩包，仍保留在根目录，但已被 Git 忽略。可以将 EXE 单独上传到发布页面；不必提交到源码仓库。

最省事的开源源码交付文件是 `release/MOD Manager-2.0.0-source.zip`，已排除本机开发环境。若手动上传整个目录，请排除 `_local` 和 `release`；Git 的忽略规则不会替普通文件上传工具自动排除文件。

继续本机开发：`scripts/restore-local-dependencies.ps1` 可将缓存依赖放回 `node_modules`；也可重新运行 `npm ci`。本机辅助构建脚本已使用新的工具链及 Rust 缓存位置。正常的干净源码 checkout 不包含 `_local`，使用开发者自己的标准环境即可。
