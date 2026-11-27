# FM27 CA Growth

两个独立的 Windows 窗口程序，为《Football Manager 27》中**人类主教练执教的球队**提供每月 CA 增长和阵容缺口分析。

它们通过 [`fmlibre-core`](crates/fmlibre-core) 直接读取正在运行的游戏，不使用存档文件，也不依赖 Tauri、React 或 npm。

> 修改游戏内存前请先备份存档。建议只用于离线单人游戏。

## 功能

- **自动连接、自动监听**：增长窗口启动后自动找到 FM27，每个新的游戏月份写入一次 CA。
- **年龄分档上限**：CA 不超过 PA 的 60% / 90% / 100%，按年龄切换。
- **阵容缺口**：独立窗口显示 ACA/APA、阵型深度、薄弱位置、内部覆盖和可出售球员。
- **省心的桌面体验**：设置自动保存，中文名正常显示，日志记录每次结算。

## 快速开始

1. 启动 FM27，并载入一个存档。
2. 运行 `fm27-growth-gui.exe` 开始每月 CA 增长。
3. 运行 `fm27-squad-gui.exe` 查看阵容缺口。

两个程序共用 `%APPDATA%\fm27-growth\settings.json`。增长窗口自动开始监听；日期倒退时它会报错并停止，点 `Resume watching` 恢复。

## 文档

- [增长规则](docs/growth/rules.md)
- [阵容窗口规则](docs/growth/squad.md)
- [使用说明](docs/growth/usage.md)
- [构建与发布](docs/growth/build.md)
- [改动清单与文件说明](docs/growth/implementation.md)

## 从源码构建

```powershell
cargo run -p fm27-growth-gui
cargo run -p fm27-squad-gui
cargo build -p fm27-growth-gui -p fm27-squad-gui --release
```

生成文件：`target\release\fm27-growth-gui.exe` 和 `target\release\fm27-squad-gui.exe`。详见[构建与发布](docs/growth/build.md)。

## 许可

基于 [FMLibreEditor](https://github.com/tozmoz/FMLibreEditor)，复用其 `fmlibre-core` 与 FM27 profile/layout 数据。许可证与上游一致：GPL-3.0-or-later。
