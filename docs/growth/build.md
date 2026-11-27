# 构建与发布

## 环境

- Rust 工具链（`rust-version = 1.90`）；
- Windows 构建需要 MSVC 工具链；
- FM27 对应版本的 layout/profile 已包含在仓库中，无需额外准备。

## 开发运行

```powershell
cargo run -p fm27-growth-gui
```

## Release 构建

```powershell
cargo build -p fm27-growth-gui --release
```

生成：

```text
target\release\fm27-growth-gui.exe
```

体积相关配置：

- 根 `Cargo.toml` 的 `[profile.release]`：`lto = true`、`codegen-units = 1`、`strip = true`、`panic = "abort"`；
- `eframe` 关闭默认特性，只启用 `default_fonts` 和 `glow`，不使用 webview；
- 根 `Cargo.toml` 的 `[profile.dev]`：`debug = 0`，保留增量编译，本地 `target` 更小。

生成的 exe 是单文件，profile/layout 已编译进 `fmlibre-core`，不需要附带其他文件。

构建前请先关闭正在运行的程序，否则 Windows 会因覆盖 exe 报“拒绝访问”。

## 检查

```powershell
cargo fmt -p fm27-growth-gui
cargo test -p fm27-growth-gui
cargo clippy -p fm27-growth-gui --all-targets -- -D warnings
```

## 分支

| 分支 | 用途 |
| --- | --- |
| `main` | 主分支：日常开发和 `growth-v*` 发布标签都打在这里 |
| `upstream` | 上次同步的上游提交，只用于合并上游更新 |

`upstream` 当前指向上游基线 `3cb3566`。同步上游时先更新这个分支，再合并进 `main`：

```powershell
git fetch upstream
git checkout upstream
git reset --hard upstream/main
git checkout main
git merge upstream
```

如果还没有配置上游 remote：

```powershell
git remote add upstream https://github.com/tozmoz/FMLibreEditor.git
```

## 发布

仓库只有一个工作流：

```text
.github/workflows/growth-release.yml
```

推送 `growth-v*` 标签即可检查、构建并创建草稿 Release：

```powershell
git tag growth-v0.1.0
git push origin growth-v0.1.0
```

工作流会依次：

1. `cargo fmt -p fm27-growth-gui -- --check`
2. `cargo clippy -p fm27-growth-gui --all-targets -- -D warnings`
3. `cargo test -p fm27-growth-gui`
4. `cargo build -p fm27-growth-gui --release`
5. 打包成 zip；
6. 上传 artifact；
7. 创建 draft GitHub Release 并附加 zip。

在 Actions 页面手动运行该工作流时，同样会做检查和构建，但只保留 artifact，不会创建 Release。

如果创建 Release 时报权限错误，到 **Settings → Actions → General → Workflow permissions** 选择 **Read and write permissions**。

## 为什么只有一个工作流

这个 fork 只发布 growth 窗口，不发布编辑器，所以删除了上游的 `ci.yml` 和 `release.yml`，把它们原本的检查移进了 `growth-release.yml`。

上游 `release.yml` 本来也不能直接用：

- 只由 `v*` 触发，并要求标签等于编辑器版本 `app/src-tauri/tauri.conf.json`；
- Windows 安装包需要签名私钥 `TAURI_SIGNING_PRIVATE_KEY`，fork 没有该 secret 时标签构建会失败；
- 发布任务只接受固定文件名，不含 `fm27-growth-gui.exe`；
- 发布前会等待该提交上的 `ci.yml` 通过。

现在只保留一个工作流：标签触发时先检查再构建，手动触发时只检查和构建，不创建 Release。
