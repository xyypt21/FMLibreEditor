# FM27 CA Growth

一个独立的 Windows 窗口程序，为《Football Manager 27》中**人类主教练执教的球队**提供每月 CA 增长。

它通过 [`fmlibre-core`](crates/fmlibre-core) 直接读取正在运行的游戏，不使用存档文件，也不依赖 Tauri、React 或 npm。

> 修改游戏内存前请先备份存档。建议只用于离线单人游戏。

---

## 功能

- **自动连接**：启动后自动寻找正在运行的 FM27，找不到时每 3 秒重试。
- **自动监听**：连接并能读到游戏日期后，自动开始监听游戏月份。
- **每月增长**：每个新的游戏月份，为受控球队中符合规则的球员增加 CA。
- **只读预览**：启动时自动生成一次预览；之后可点 `Preview next settlement` 手动刷新。
- **年龄上限**：按年龄分档限制 CA 不超过 PA 的一定比例。
- **设置保存**：关闭窗口时自动保存，下次启动自动读取。
- **中文显示**：自动加载 Windows 中文字体，俱乐部和球员中文名不会显示成方块。
- **日志面板**：连接信息、结算数量、写入失败原因都记录在窗口底部。

---

## 增长规则

### 1. 每月增长

每进入一个新的游戏月份，对受控球队中每个可增长球员执行一次：

```text
新 CA = 当前 CA + Monthly CA gain
```

### 2. 年龄分档上限

默认设置：

| 年龄 | CA 上限 |
| --- | --- |
| 18.0 及以下 | PA × 0.6 |
| 18.1 – 20.9 | PA × 0.9 |
| 21.0 及以上 | PA 本身 |

特点：

- 不再是线性插值，而是三个固定档位；
- 只有整 18.0 仍在第一档，18.1 起进入第二档；
- 到 21.0 直接使用 PA；
- 上限四舍五入。

这些值都可以在窗口左侧修改：

```text
Age 1 / Ratio 1
Age 2 / Ratio 2
Max age
Monthly CA gain
```

### 3. 停止增长

`Max age` 默认 23：

- 22.9 岁仍然可以增长；
- 23.0 岁起完全停止。

### 4. 其他限制

- CA 和 PA 必须在 `1..=200`；
- CA 不能高于 PA；
- 已经达到 PA 的球员不增长；
- 如果 `CA + 月增长` 会超过年龄上限，该球员本次跳过，**不会裁剪到上限**；
- FM 中 PA 为负数区间时视为无效记录，跳过。

### 5. 写入时机

- 启动 watcher 本身不写 CA；
- 只有游戏月份真正前进时才结算；
- 载入存档不会触发增长，会重置月份状态；
- 预览永远只读。

---

## 使用

1. 启动 FM27，并载入一个存档。
2. 运行 `fm27-growth-gui.exe`。
3. 程序会自动连接并开始监听。
4. 修改左侧设置，点 `Preview next settlement` 查看计划。
5. 游戏内点 **Continue** 推进月份，程序会在新月份写入 CA。
6. 关闭窗口时会保存设置。

设置文件位置：

```text
%APPDATA%\fm27-growth\settings.json
```

没有 `APPDATA` 时保存到当前目录的 `fm27-growth-settings.json`。

---

## 从源码构建

需要 Rust 工具链，以及 FM27 对应版本的 layout/profile（仓库中已包含）。

```powershell
cargo run -p fm27-growth-gui
```

Release 构建：

```powershell
cargo build -p fm27-growth-gui --release
```

生成文件：

```text
target\release\fm27-growth-gui.exe
```

测试与检查：

```powershell
cargo fmt -p fm27-growth-gui
cargo test -p fm27-growth-gui
cargo clippy -p fm27-growth-gui --all-targets -- -D warnings
```

---

## 发布

仓库包含独立的工作流：

```text
.github/workflows/growth-release.yml
```

推送 `growth-v*` 标签即可构建并创建草稿 Release：

```powershell
git tag growth-v0.1.0
git push origin growth-v0.1.0
```

工作流会：

1. 在 Windows 上构建 `fm27-growth-gui.exe`；
2. 打包成 zip；
3. 创建 draft GitHub Release 并附加 zip。

在 Actions 页面手动运行该工作流时，只会生成 artifact，不会创建 Release。

### 为什么不直接复用上游的 CI / Release

**CI 可以复用。** 上游 `ci.yml` 执行的是：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`fm27-growth-gui` 是 workspace 成员，因此格式、clippy 和测试都会被自动检查，不需要额外的 CI 文件。

**Release 不能直接复用。** 上游 `release.yml`：

- 只由 `v*` 标签触发，并要求标签等于编辑器版本 `app/src-tauri/tauri.conf.json`，本程序无法使用独立版本号；
- Windows 安装包需要签名私钥 `TAURI_SIGNING_PRIVATE_KEY`，fork 没有该 secret 时标签构建会失败（只有 PR 允许无签名构建）；
- 发布任务只接受固定文件名，不含 `fm27-growth-gui.exe`，要加入就必须修改上游文件，后续合并上游会产生冲突；
- 发布前会等待该提交上的 `ci.yml` 通过，而标签提交通常没有跑过 CI。

因此发布保持为独立工作流 `growth-release.yml`，与上游编辑器发布互不影响。

---

## 实现方式

- GUI：Rust + `eframe`/`egui`；
- 游戏读取：`fmlibre-core` 的定向接口，只读取训练管理器中人类主教练的俱乐部和球队，不扫描全量 staff、俱乐部数组或球员数组；
- 增长规则与结算：`crates/fm27-growth-gui/src/growth.rs`；
- 后台连接与月份监听：`crates/fm27-growth-gui/src/worker.rs`；
- 界面：`crates/fm27-growth-gui/src/main.rs`。

---

## 与上游的关系

本项目基于 [FMLibreEditor](https://github.com/tozmoz/FMLibreEditor)，复用其 `fmlibre-core` 与 FM27 profile/layout 数据。

FM27 CA Growth 只增加了一个独立 GUI crate 和少量公开的定向读取接口，便于继续合并上游更新。

许可证与上游一致：GPL-3.0。
