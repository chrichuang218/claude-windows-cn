# 贡献指南

感谢你愿意为 Claude 中文助手贡献力量！在开始之前，请先阅读以下约定。

## 行为准则

请保持友善、尊重、专业。任何形式的骚扰与人身攻击均不被接受。

## 开发环境准备

需要安装：

- Node.js（20.19+ 的 20.x，或 22.12+）
- Rust（当前 stable 工具链）
- Windows：Microsoft C++ Build Tools 和 Microsoft Edge WebView2 Runtime
- macOS 13+：Xcode Command Line Tools（`xcode-select --install`）
- macOS 汉化与恢复：Python 3.9+（python.org 或 Homebrew）；缺少依赖时助手提示安装，不自动修改开发环境

```powershell
git clone https://github.com/chrichuang218/claude-desktop-cn.git
cd claude-desktop-cn
npm ci
```

## 常用命令

```powershell
# 启动前端 + Tauri 开发模式（热重载）
npm run tauri dev

# 仅启动前端预览（浏览器，用于调整界面）
npm run dev

# 类型检查 + 生产构建
npm run build

# 代码检查
npm run lint

# 检查 Rust 后端
cargo check --manifest-path src-tauri/Cargo.toml

# 检查格式与后端单元测试
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml

# 汉化适配器隔离测试（两端均可运行，不操作真实 Claude）
python -m unittest discover -s tests -p "test_macos_engine_adapter.py" -v
```

Windows 打包运行 `npm run tauri build -- --no-bundle`；macOS 打包运行 `APPLE_SIGNING_IDENTITY=- npm run tauri build -- --bundles app`。后者生成本地签名 `.app`，不代表已通过 Apple 公证。CI 分别使用 Windows x64、macOS arm64 和 macOS x64 runner 构建、运行测试、自检并上传带 SHA256 摘要的包，不自动发布 Release。

前端和 Tauri 命令保持同一套接口，操作系统差异在 Rust 平台模块内实现。助手的 `assistantPath` 均表示安装目录；macOS 在其下放置 `Claude 中文助手.app`。应用标识、配置产品标识和 Windows 发行文件名保持兼容，不能因支持 macOS 顺手改名。

## 提交规范

- 使用清晰的提交信息，建议格式：`类型: 简述`，如 `feat: 新增深色模式`、`fix: 修复刷新闪烁`、`docs: 更新 README`。
- 一个 PR 聚焦一件事，便于评审与回滚。
- 提交前请确认 `npm run lint`、`npm run build`、`cargo fmt --check`、`cargo test` 与 `cargo check` 均通过；Rust 命令使用 `--manifest-path src-tauri/Cargo.toml`。

## 汉化引擎

应用和恢复汉化时，助手会在线确认 `javaht/claude-desktop-zh-cn` 的最新引擎修订。远端确认未变，且本地缓存来源与 SHA256 校验通过时复用缓存；内容变化或缓存损坏时重新下载。引擎修订独立于 Claude 版本，网络失败不能静默使用旧缓存。每次操作仍独立解压并检查、适配脚本。

开发时仅预览界面不需要下载引擎；验证真实补丁操作会修改本机 Claude Desktop，须先确认测试对象和范围。提交验证结果时，请区分隔离目录测试、真实 Windows 注册包 / macOS `.app` 操作与 Cowork 实际功能，不将其中一项当作全部验证通过。macOS 发布前按 [验收清单](docs/macos-acceptance.md) 分别核验 Apple Silicon 和 Intel，不把 CI 通过写成实机业务验收通过。

## 国际化与文案

当前以中文用户为主要受众，界面文案以内联中文为主，暂不引入 i18n 框架。修改面向用户的文案时请保持措辞简洁、友好。

## 提交 Issue / PR

- Bug 反馈请使用 Issue 模板，附上系统版本、复现步骤与执行页日志截图。
- 功能建议请说明使用场景与预期效果。
- PR 请关联对应 Issue，并描述实现思路与测试方式。
