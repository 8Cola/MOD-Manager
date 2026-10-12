# 测试说明

2026-10-12 开源发布前重新运行前端类型检查／生产构建和 Windows Rust 回归测试：96 项通过、0 失败，另有一个默认忽略的耗时探针，可独立运行。GitHub Actions 会在 main 推送及 Pull Request 时检查前端构建、Rust 格式和核心回归。

```powershell
npm ci
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
```

测试通过临时目录验证后缀启停、文件夹清单、多文件分组、同名冲突、文件占用回滚、符号链接／目录联接保护、游戏运行阻止、恢复日志、删除与恢复、SQLite 重启、导入导出及原图保留。耗时比较及原始数据见 `TOGGLE-PERFORMANCE.md`。

`scripts/qa-*.cjs` 是本地 Windows WebView 验收脚本，不属于普通启动依赖。运行需要 Playwright、已构建的 Debug EXE 和 Vite 开发服务。可通过 `PLAYWRIGHT_MODULE_PATH` 指定 Playwright 模块，个别脚本需要 Python，可通过 `QA_PYTHON` 指定。Debug 专用 `MM_TEST_DATA_DIR` 隔离数据库和图片；Release 不启用此覆盖或自动调试端口。

验收文件、截图和报告写入 Git 忽略的 `_local/`。部分脚本记录早期界面的验收方式，界面改变后可能需要更新选择器；最新启停、原图与手动保存流程对应 `qa-toggle-speed.cjs`、`qa-original-cover.cjs`、`qa-manual-notes.cjs`。请始终使用独立目录，避免操作真实 Steam MOD。
