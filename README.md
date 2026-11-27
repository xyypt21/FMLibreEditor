# FM27 CA Growth

一个独立的 Windows 窗口程序，为《Football Manager 27》中**人类主教练执教的球队**提供每月 CA 增长。

它通过 [`fmlibre-core`](crates/fmlibre-core) 直接读取正在运行的游戏，不使用存档文件，也不依赖 Tauri、React 或 npm。

> 修改游戏内存前请先备份存档。建议只用于离线单人游戏。

## 功能

- **自动连接、自动监听**：启动后自动找到 FM27，每个新的游戏月份写入一次 CA。
- **年龄分档上限**：CA 不超过 PA 的 60% / 90% / 100%，按年龄切换。
- **只读预览**：启动时预览一次，之后可手动刷新。
- **省心的桌面体验**：设置自动保存，中文名正常显示，日志记录每次结算。

## 快速开始

1. 启动 FM27，并载入一个存档。
2. 运行 `fm27-growth-gui.exe`。
3. 程序自动连接并开始监听；游戏内推进月份时写入 CA。

设置保存在 `%APPDATA%\fm27-growth\settings.json`。

## 文档

- [增长规则](docs/growth/rules.md)
- [使用说明](docs/growth/usage.md)
- [构建与发布](docs/growth/build.md)
- [改动清单与文件说明](docs/growth/implementation.md)

## 从源码构建

```powershell
cargo run -p fm27-growth-gui
cargo build -p fm27-growth-gui --release
```

生成文件：`target\release\fm27-growth-gui.exe`。详见[构建与发布](docs/growth/build.md)。

## 许可

基于 [FMLibreEditor](https://github.com/tozmoz/FMLibreEditor)，复用其 `fmlibre-core` 与 FM27 profile/layout 数据。许可证与上游一致：GPL-3.0-or-later。
