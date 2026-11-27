# 改动清单与文件说明

相对上游基线 `3cb3566`，本工具新增了两个独立 GUI crate 和一个共享规则 crate，并在 `fmlibre-core` 中公开了少量定向读取接口。没有修改任何内存偏移常量，也没有新增写接口。

## 改了哪些东西

| 类别 | 内容 |
| --- | --- |
| 新 crate | `crates/fm27-growth-model`：增长、年龄、ACA/APA、阵型与出售规则 |
| 新 crate | `crates/fm27-growth-gui`：增长窗口、监听线程 |
| 新 crate | `crates/fm27-squad-gui`：阵容窗口、定时读取线程 |
| core 新增接口 | `ClubIdentity`、`Session::club_at`、`Session::human_clubs`、`TeamPlayer`、`team_objects_at`、`linked_team_addresses`、`team_players` |
| 工作区配置 | 根 `Cargo.toml` 增加 `eframe`、`fm27-growth-model` 依赖与 `[profile.dev]` |
| 发布 | `.github/workflows/growth-release.yml` 同时构建两个 exe，删除上游的 `ci.yml` 与 `release.yml` |
| 文档 | `README.md` 重写为简介，详细内容移到 `docs/growth/` |
| 依赖锁 | `Cargo.lock` 随新依赖更新 |

## 文件清单

```text
.github/workflows/growth-release.yml     两个 exe 的检查、构建与发布
.github/workflows/ci.yml                  删除
.github/workflows/release.yml             删除
Cargo.toml                                新增共享规则 crate 与 eframe
crates/fm27-growth-model/                 规则层：settings / date / squad
crates/fm27-growth-gui/                   增长窗口：growth / main / worker
crates/fm27-squad-gui/                    阵容窗口：main / worker
crates/fmlibre-core/src/clubs.rs          human_clubs
crates/fmlibre-core/src/squad.rs          TeamPlayer 的位置、球队与雇主球队
crates/fmlibre-core/src/lib.rs            导出
README.md / docs/growth/*                 文档
```

## 共享规则 crate：`crates/fm27-growth-model`

两个窗口共用同一份规则，避免增长窗口和阵容窗口各算一套。它不读内存、不开窗口，测试可以不启动游戏。

```toml
chrono         年龄的日历计算
serde          设置 JSON
serde_json     设置文件读写
```

| 文件 | 内容 |
| --- | --- |
| `settings.rs` | `CapRule`、`GrowthSettings`、`validate`、`month_key`、`growth_cap`、`growth_target`、设置文件读写 |
| `date.rs` | `Ymd` 与 `age_tenths_on`，用生日和游戏日期算 `岁 + 十分位` |
| `squad.rs` | ACA/APA、阵型解析、深度配额、选人、地板/覆盖、出售规则 |

`GrowthSettings` 在原来的 `monthly_gain`、`max_age`、`cap` 之外新增：

```text
apa_shortfall_years   默认 2.0，APA 最多预测多少年
aca_growth_years      默认 0.5，ACA 现在展示多少年
formation             默认 4-4-2 深度阵型文本
```

配置文件由两个窗口共享，`update_settings` 会先读回最新文件再改自己的字段，所以同时打开不会互相覆盖。

## 增长窗口：`crates/fm27-growth-gui`

### `Cargo.toml`

```toml
eframe             窗口（glow 渲染，无 webview）
fmlibre-core       游戏读取与写入
fmlibre-mem        指针合法性判断
fm27-growth-model  共享规则
```

### `src/growth.rs`：计划与结算

规则本身来自 `fm27-growth-model`，这里只保留窗口侧需要读写的部分。

| 内容 | 说明 |
| --- | --- |
| `month_key` | 把 `FmDate` 转成 `年 × 12 + 月` |
| `plan_growth` | 用 `Session::human_clubs` 遍历俱乐部球队、关联球队（B 队），按 UID 去重，生成计划，只读 |
| `settle` | 按计划里的 `address`/`shift` 直接写 CA，并回读校验；玩家已移动或 CA 已变化则跳过 |
| `GrowthChange` | 计划中的一行，带写入所需的地址与人员块偏移 |

关键点：

- 球员只从受控俱乐部读取，不扫描 staff、俱乐部数组或全量球员数组；
- 年龄用十分位参与比较，所以 18.0 与 18.1、22.9 与 23.0 的边界是准确的；
- `address` 和 `shift` 只存在于 Rust 结构里，不参与序列化，GUI 关闭后也不会保存到设置文件。

### `src/main.rs`：窗口

| 内容 | 说明 |
| --- | --- |
| 设置 | 通过 model 的 `load_settings` / `update_settings` 读写 `%APPDATA%\fm27-growth\settings.json` |
| `install_fonts` | 依次尝试 `msyh.ttc`、`simhei.ttf`、`simsun.ttc`，解决中文方块 |
| `App` | 左侧设置与按钮，右侧状态、计划表格、日志 |
| 设置同步 | 数值变化时发送 `Command::Settings`，正在运行的 watcher 立即使用新规则 |
| 日期倒退 | 显示错误与 `Resume watching` 按钮，watcher 停止后不会自动重启 |
| 退出 | `on_exit` 只保存增长窗口自己的字段，保留阵容窗口的 `formation` |

### `src/worker.rs`：后台线程

窗口线程不直接读内存，所有 `Process`/`Session` 操作都在这个线程里。

| 内容 | 说明 |
| --- | --- |
| `Command` | `Preview`、`Settings`、`Resume` |
| `Event` | `Log`、`Connected`、`Plan`、`Running`、`Stopped` |
| `spawn` | 建立双向通道，启动名为 `fm27-growth` 的线程 |
| 自动连接 | 首次立即尝试，失败每 3 秒重试 |
| 首次预览 | 连接成功后自动预览一次，之后只由按钮触发 |
| 自动监听 | 能读到游戏日期即启动 watcher，否则每 2 秒重试 |
| `decide` | 计数变化 → 读档重置；月份相同 → 空闲；月份前进 → 结算；月份倒退且未读档 → 报错并停止 |
| 错误节流 | 相同错误只记录一次，成功后清空 |

## 阵容窗口：`crates/fm27-squad-gui`

```toml
chrono             本地时间，用于状态行的更新时间
eframe             窗口
fmlibre-core       受控俱乐部、球队和球员读取
fmlibre-mem        启动进程
fm27-growth-model  ACA/APA、阵型、缺口与出售规则
```

### `src/worker.rs`

| 内容 | 说明 |
| --- | --- |
| `Command` | `Refresh`、`Pause`、`Settings` |
| `Event` | `Status`、`Connected`、`Report` |
| `read_squad` | 读受控俱乐部、所属球队、关联 B 队，按 UID 去重；按 `TeamPlayer.team` 是否在本队集合判断租借在外 |
| 自动刷新 | 连接后立即读一次，之后每 30 秒一次 |
| 编辑阵型 | `Command::Settings` 直接用上次读到的球员重新计算，不重新读内存 |
| 只读 | 不调用任何写入接口 |

### `src/main.rs`

| 内容 | 说明 |
| --- | --- |
| 顶部 | `Pause` / `Resume`、`Refresh now`、`Squad rules`、状态行 |
| 阵型 | 文本框实时校验，必须 11 首发；`Reset` 恢复默认 4-4-2 |
| Positions | 位置、已填/配额、最高 APA、最低 APA、状态；缺口红色、覆盖绿色 |
| Sell candidates | 姓名、年龄、CA、PA、ACA、APA、原因 |
| 退出 | `on_exit` 只保存 `formation` |

## `fmlibre-core` 的改动

### `src/clubs.rs`

新增：

```rust
pub struct ClubIdentity {
    pub uid: u32,
    pub name: String,
    #[serde(skip)]
    pub address: u64,
}

pub fn club_at(&self, address: u64) -> Option<ClubIdentity>
```

训练管理器保存的是俱乐部指针，不是俱乐部数组的下标。`club_at` 直接按指针读取 UID 与名称，并校验：

- 指针属于用户态；
- UID 不为 0；
- 名称指针可读且看起来像俱乐部名。

任一项不满足返回 `None`，由调用方跳过。

### `src/squad.rs`

新增：

```rust
pub struct TeamPlayer {
    pub uid: u32,
    pub name: String,
    pub date_of_birth: Option<FmDate>,
    pub current_ability: u16,
    pub potential_ability: i16,
    #[serde(skip)] pub address: u64,
    #[serde(skip)] pub shift: u64,
}
```

只包含增长需要的字段，避免构造完整球员档案；`address`/`shift` 用于结算时直接写 CA。

| 方法 | 作用 |
| --- | --- |
| `team_objects_at(club, uid)` | 从俱乐部对象读球队列表；校验指针、UID、球队数量上限、每支球队反向指回俱乐部 |
| `linked_team_addresses(club)` | 读关联球队（B 队），过滤掉不属于其他俱乐部的条目 |
| `team_players(team)` | 按球队地址读球员列表，逐个读取 UID、姓名、生日、CA、PA |
| `team_player` | 私有辅助：读单个球员；内部缓存各对象类的人员块偏移，避免重复探测 |

`team_objects` 由原来的完整实现改为委托：

```rust
self.team_objects_at(club.address, club.uid)
```

俱乐部数组路径与指针路径因此共用同一份校验逻辑，行为不变。

### `src/lib.rs`

导出 `ClubIdentity` 与 `TeamPlayer`，GUI 才能直接 `use fmlibre_core::{ClubIdentity, TeamPlayer}`。

## 其他文件

### 根 `Cargo.toml`

```toml
eframe = { version = "0.37", default-features = false, features = ["default_fonts", "glow"] }
```

统一依赖版本；同时新增：

```toml
[profile.dev]
debug = 0
incremental = true
```

本地编译不生成调试信息，`target` 更小，但仍然增量编译。

### `.github/workflows/growth-release.yml`

仓库唯一的工作流。`growth-v*` 标签或手动运行时依次执行 `fmt`、`clippy`、`test`、release 构建；标签运行还会打包 zip、创建 draft Release，手动运行只上传 artifact。

上游的 `ci.yml` 与 `release.yml` 已删除：这个 fork 只发布 growth 窗口，原先由它们执行的检查移入了本工作流。详见[构建与发布](build.md)。

### `README.md` 与 `docs/growth/`

README 只保留简介、功能、快速开始和文档入口；规则、使用、构建发布、本文件放在 `docs/growth/`。

## 没有改什么

- 没有新增或修改 layout 偏移常量；
- 没有新增写内存的公开 API，`settle` 使用 core 已有的写入能力；
- 没有改动上游编辑器的 Tauri 界面；
- `fmlibre-core` 的新增方法全部只读，原有 218 个测试保持通过。

## 合并上游时

需要留意的是三处 core 文件、根 `Cargo.toml`，以及两个被删除的工作流文件：

```text
crates/fmlibre-core/src/clubs.rs
crates/fmlibre-core/src/squad.rs
crates/fmlibre-core/src/lib.rs
Cargo.toml
.github/workflows/ci.yml
.github/workflows/release.yml
```

删除上游工作流会在合并上游时表现为 modify/delete 冲突，保留删除即可。`fm27-growth-gui` 是独立 crate，`growth-release.yml` 是独立工作流，通常不会与上游冲突。
