# 参与开发

欢迎通过 Issue 提交问题或通过 Pull Request 改进项目。提交问题时请说明 Windows 版本、软件版本、MOD 是文件还是文件夹、操作步骤和实际提示；日志与截图请先去掉个人路径和私有内容。

开发环境与命令见 README。提交前运行 `npm run build`、`cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` 和 `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib`。

涉及文件操作的修改请先使用独立测试目录验证：冲突不得覆盖，失败须如实反馈，恢复日志不可省略；不要用真实游戏目录做自动化测试。

不要提交 `_local`、`node_modules`、编译产物、真实 MOD、用户数据库或凭据。项目功能以简洁的本地管理为目标，新功能请先讨论需求与兼容性。
