# MOD Manager 标志

正式标志为蓝色玻璃底座上的分体游戏方块：珠光顶面、十字键左面和双按键右面。

唯一源文件是 `src/assets/app-icon.svg`，由 SVG 路径、仿射变换、渐变、遮罩和光影代码重绘，不嵌入用户参考 PNG，也不依赖外部图片或字体。侧栏与“关于”页面直接使用此源图。

运行 `npm run icons`，通过同一源图生成 `src-tauri/icons/icon.png` 和包含 16、20、24、32、40、48、64、96、128、256 像素帧的 Windows ICO。更换标志时应修改 SVG 后重新生成资源，不能单独替换其中一份。

Windows 构建会把 ICO 嵌入 EXE。可运行 `python scripts/verify_icon.py path/to/mod-manager.exe`，逐帧核对原生资源与生成 ICO 的 PNG 字节完全相同。Windows 桌面图标缓存需在交付新 EXE 后通知系统刷新；不要依赖界面图标变化推断桌面资源已更新。
