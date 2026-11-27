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

## 发布

仓库包含独立工作流：

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

如果创建 Release 时报权限错误，到 **Settings → Actions → General → Workflow permissions** 选择 **Read and write permissions**。

## 为什么不直接复用上游的 CI / Release

**CI 可以复用。** 上游 `ci.yml` 执行：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`fm27-growth-gui` 是 workspace 成员，格式、clippy 和测试都会被自动检查，不需要额外的 CI 文件。

**Release 不能直接复用。** 上游 `release.yml`：

- 只由 `v*` 触发，并要求标签等于编辑器版本 `app/src-tauri/tauri.conf.json`，本程序无法使用独立版本号；
- Windows 安装包需要签名私钥 `TAURI_SIGNING_PRIVATE_KEY`，fork 没有该 secret 时标签构建会失败（只有 PR 允许无签名构建）；
- 发布任务只接受固定文件名，不含 `fm27-growth-gui.exe`，要加入就必须修改上游文件，后续合并上游会产生冲突；
- 发布前会等待该提交上的 `ci.yml` 通过，而标签提交通常没有跑过 CI。

因此发布保持为独立工作流 `growth-release.yml`，与上游编辑器发布互不影响。
