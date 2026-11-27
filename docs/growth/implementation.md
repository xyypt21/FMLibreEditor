# 改动清单与文件说明

相对上游基线 `3cb3566`，本工具新增了一个独立 GUI crate，并在 `fmlibre-core` 中公开了少量定向读取接口。没有修改任何内存偏移常量，也没有新增写接口。

## 改了哪些东西

| 类别 | 内容 |
| --- | --- |
| 新 crate | `crates/fm27-growth-gui`：窗口、增长规则、后台线程 |
| core 新增接口 | `ClubIdentity`、`Session::club_at`、`TeamPlayer`、`team_objects_at`、`linked_team_addresses`、`team_players` |
| 工作区配置 | 根 `Cargo.toml` 增加 `eframe` 依赖与 `[profile.dev]` |
| 发布 | 新增 `.github/workflows/growth-release.yml` |
| 文档 | `README.md` 重写为简介，详细内容移到 `docs/growth/` |
| 依赖锁 | `Cargo.lock` 随新依赖更新 |

## 文件清单

```text
.github/workflows/growth-release.yml      +55
Cargo.toml                                 +8
crates/fm27-growth-gui/Cargo.toml         +16
crates/fm27-growth-gui/src/growth.rs     +386
crates/fm27-growth-gui/src/main.rs       +365
crates/fm27-growth-gui/src/worker.rs     +290
crates/fmlibre-core/src/clubs.rs          +27
crates/fmlibre-core/src/lib.rs             +6
crates/fmlibre-core/src/squad.rs         +140
README.md / docs/growth/*                 文档
```

## 新增 crate：`crates/fm27-growth-gui`

### `Cargo.toml`

```toml
chrono         日期与月份计算
eframe         窗口（glow 渲染，无 webview）
fmlibre-core   游戏读取与写入
fmlibre-mem    指针合法性判断
serde          /
serde_json     设置的读写
```

`eframe` 在根 `Cargo.toml` 中统一定义，关闭默认特性，只保留 `default_fonts` 和 `glow`，因此不会引入 WebView，编译和体积都更小。

### `src/growth.rs`：规则与结算

这是唯一包含增长业务逻辑的文件。

| 内容 | 说明 |
| --- | --- |
| `CapRule` | `age1/ratio1/age2/ratio2`，默认 `18 / 0.6 / 21 / 0.9` |
| `GrowthSettings` | `monthly_gain`、`max_age`、`cap`，带 serde 默认值 |
| `validate` | 月增长 > 0、`age1 < age2`、比例在 `(0, 1]` |
| `month_key` | `年 × 12 + 月`，用于判断月份是否变化 |
| `growth_cap` | 三档上限：`age <= age1` 用 `ratio1`，`age < age2` 用 `ratio2`，否则 PA；结果四舍五入 |
| `growth_target` | 校验 CA/PA 范围、`max_age`、上限后返回新 CA；超上限时返回 `None`，不裁剪 |
| `controlled_clubs` | 从训练管理器读取人类主教练，再经 `club_at` 得到俱乐部 |
| `plan_growth` | 遍历俱乐部球队、关联球队（B 队），按 UID 去重，生成计划，只读 |
| `settle` | 按计划里的 `address`/`shift` 直接写 CA，并回读校验；玩家已移动或 CA 已变化则跳过 |
| `age_tenths_on` | 用生日和游戏日期计算 `岁 + 十分位` 的年龄 |

关键点：

- 球员只从受控俱乐部读取，不扫描 staff、俱乐部数组或全量球员数组；
- 年龄用十分位参与比较，所以 18.0 与 18.1、22.9 与 23.0 的边界是准确的；
- `address` 和 `shift` 只存在于 Rust 结构里，不参与序列化，GUI 关闭后也不会保存到设置文件。

### `src/main.rs`：窗口

| 内容 | 说明 |
| --- | --- |
| `settings_path` | `%APPDATA%\fm27-growth\settings.json`；无 `APPDATA` 时用当前目录 |
| `load_settings` / `save_settings` | JSON 读写；`on_exit` 时保存 |
| `install_fonts` | 依次尝试 `msyh.ttc`、`simhei.ttf`、`simsun.ttc`，解决中文方块 |
| `App` | 左侧设置与按钮，右侧状态、计划表格、日志 |
| `on_event` | 把 worker 的事件转成界面状态与日志 |
| 设置同步 | 数值变化时发送 `Command::Settings`，正在运行的 watcher 立即使用新规则 |
| 退出 | `on_exit` 保存设置；worker 线程随进程结束 |

### `src/worker.rs`：后台线程

窗口线程不直接读内存，所有 `Process`/`Session` 操作都在这个线程里。

| 内容 | 说明 |
| --- | --- |
| `Command` | `Preview`、`Settings` |
| `Event` | `Log`、`Connected`、`Plan`、`Running` |
| `spawn` | 建立双向通道，启动名为 `fm27-growth` 的线程 |
| 自动连接 | 首次立即尝试，失败每 3 秒重试 |
| 首次预览 | 连接成功后自动预览一次，之后只由按钮触发 |
| 自动监听 | 能读到游戏日期即启动 watcher，否则每 2 秒重试 |
| `tick` | 先用“已载入存档次数”识别读档并重置；月份变化时结算，日期不参与读档判断 |
| 错误节流 | 相同错误只记录一次，成功后清空 |

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

在 `growth-v*` 标签上构建 Windows exe、打包 zip、创建 draft Release；手动运行只上传 artifact。与上游编辑器发布分开，详见[构建与发布](build.md)。

### `README.md` 与 `docs/growth/`

README 只保留简介、功能、快速开始和文档入口；规则、使用、构建发布、本文件放在 `docs/growth/`。

## 没有改什么

- 没有新增或修改 layout 偏移常量；
- 没有新增写内存的公开 API，`settle` 使用 core 已有的写入能力；
- 没有改动上游编辑器的 Tauri 界面；
- `fmlibre-core` 的新增方法全部只读，原有 218 个测试保持通过。

## 合并上游时

需要留意的只有三处 core 文件和根 `Cargo.toml`：

```text
crates/fmlibre-core/src/clubs.rs
crates/fmlibre-core/src/squad.rs
crates/fmlibre-core/src/lib.rs
Cargo.toml
```

`fm27-growth-gui` 是独立 crate，`growth-release.yml` 是独立工作流，通常不会与上游冲突。
