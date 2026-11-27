# AGENTS.md

FMLibreEditor 的分支，只发布两个独立的 Windows 工具：`fm27-growth-gui`（每月 CA 增长）和 `fm27-squad-gui`
（只读阵容深度）。`app/` 是上游 Tauri 编辑器，仅作参考，不参与发布。细节见 `docs/growth/`。

## 约束

- 本地只构建 **debug**，不用 `--release`；release 由 CI 生成。未明确要求不要推送、打标签或发布。
- 阵容窗口只读。CA 写入只能走 `fmlibre-core` 的受保护写入路径，并且必须 `writes_allowed()`。
- 只做定向读取，不扫描整个 staff、俱乐部或球员数组。偏移放在 profile/layout 数据里，不写进窗口 crate。
- 纯规则放在 `crates/fm27-growth-model` 并配单元测试。worker 持有 `Process`/`Session`；UI 线程不读游戏内存。
- 代码和界面用英文；README 与文档用简体中文。

## 命令

```powershell
cargo fmt -p fm27-growth-model -p fm27-growth-gui -p fm27-squad-gui
cargo clippy -p fm27-growth-model -p fm27-growth-gui -p fm27-squad-gui --all-targets -- -D warnings
cargo test -p fm27-growth-model -p fm27-growth-gui -p fm27-squad-gui
cargo run -p fm27-growth-gui
cargo run -p fm27-squad-gui
```

`--workspace` 只在改 `fmlibre-core`、`fmlibre-mem` 或 `app/` 时使用。

## 必须保持

- 阶梯 cap，不做插值：18.0 及以下 PA × 60%，18.1–20.9 × 90%，21.0 起用 PA。`max_age` 23 表示 22.9 可增长、
  23.0 停止。年龄按十分位比较。
- 月份 key 为 `年 × 12 + 月`，换月才结算。读档只由游戏的读档计数识别，计数变化才重置监听；月份倒退但未读档时什么都不写、
  直接忽略，不报错也不停止 watcher。
- `CA + 月增长` 超过 cap 时跳过该球员，不裁剪。
- 阵型在 Positions 表的 `Starters` 列直接编辑：可为 0，表格始终显示全部 15 个位置（0 显示 `not used`），合计
  必须为 11。GK 配额 2；其他启用位置为 `首发 × 2 + 2`。阵型启动时从
  `%APPDATA%\fm27-growth\formation.json` 读取，退出时自动保存；没有 Reset/Reload 按钮。
- 球员可踢评分 15 及以上的所有位置，按 ACA、APA 排序；租借在外排除，薄弱球员需要落选球员覆盖。已同意转出/租出的
  球员排除，已同意转入/租入的球员加入（读游戏转会经理的 state 19 提议）。
- 出售：超过 `max_age`，或超过 `max_age - apa_shortfall_years` 且 APA 不高于 floor。
- 增长设置共用 `%APPDATA%\fm27-growth\settings.json`；阵型单独存 `formation.json`。
