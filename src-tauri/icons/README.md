# 图标来源与再生成

`public/claude-icon.svg` 为界面与助手图标的矢量母版：沿用陶土底色与白色 Claude 标记。标记路径来自 [Simple Icons Claude (CC0)](https://github.com/simple-icons/simple-icons/blob/develop/icons/claude.svg)，于 2026-09-24 获取；未放大旧的 300px 截图。`app-icon.svg` 是同一母版的构建副本。

运行 `npx tauri icon public/claude-icon.svg -o src-tauri/icons` 可生成 PNG/ICNS/ICO；Windows ICO 还须保留 128px 档。当前 ICO 含 16、24、32、48、64、128、256px，来自矢量渲染后的 512px PNG 母版按尺寸降采样。`public/claude-icon.png` 保留作为 PNG 兼容资源。

Claude Desktop 的桌面快捷方式不使用助手图标，运行时从当前已验证官方包 `assets/Square150x150Logo.scale-200.png` 提取官方美术，直接按 16、24、32、48、64、128、256px 分别缩放并生成用户数据目录内的多尺寸 ICO；更新包后不会指向失效的旧 WindowsApps 目录。
