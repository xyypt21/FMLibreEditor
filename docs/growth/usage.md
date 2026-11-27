# 使用说明

## 步骤

1. 启动 FM27，并载入一个存档。
2. 运行 `fm27-growth-gui.exe`。
3. 程序自动连接并开始监听，日志会显示游戏日期和“watching”。
4. 修改左侧设置，点 `Preview next settlement` 可以查看下一次结算会改哪些球员。
5. 游戏内点 **Continue** 推进月份，程序会在新月份写入 CA。
6. 关闭窗口时自动保存设置。

## 窗口

```text
左侧                          右侧
设置                          Summary 状态行
  Monthly CA gain             计划/结算表格：球员、年龄、CA、PA、上限、变化
  Max age                     Log 日志面板
  Age 1 / Ratio 1
  Age 2 / Ratio 2
按钮
  Preview next settlement
  Growth rules
```

- 年龄显示为 1 位小数；
- 表格列出本次计划或结算的全部改动，可双向滚动；
- 日志最多保留 2000 行；
- `Growth rules` 打开规则说明窗口。

## 状态

| 状态 | 含义 |
| --- | --- |
| `waiting for the game…` | 还没有找到 FM27 |
| `connected; waiting for a save to watch` | 已连接，但读不到游戏日期（通常还在主菜单） |
| `watching the in-game month` | 正在监听，下个月份会写入 CA |

## 设置文件

```text
%APPDATA%\fm27-growth\settings.json
```

没有 `APPDATA` 时保存到当前目录的 `fm27-growth-settings.json`。

内容是 JSON；缺失字段使用默认值，因此旧版本设置可以直接读取：

```json
{
  "monthly_gain": 2,
  "max_age": 23,
  "cap": { "age1": 18, "ratio1": 0.6, "age2": 21, "ratio2": 0.9 }
}
```

## 常见问题

**一直 waiting for the game**

- 确认 FM27 正在运行；
- 程序每 3 秒重试一次，相同的错误只写一次日志；
- 若游戏版本与 profile 不匹配，日志会显示原因。

**Connected 但无法开始监听**

- 需要先载入存档，并处于可以读到游戏日期的界面；
- 程序会自动重试，不需要手动操作。

**中文显示成方块**

- 程序会依次尝试 `msyh.ttc`、`simhei.ttf`、`simsun.ttc`；
- 如果系统没有这些字体，请安装任意一款 Windows 中文字体。

**写入后数值没变**

- 已到 `Max age`、已到年龄上限或已到 PA 的球员不会变化；
- 失败原因会写在日志里，最多列出 5 条。

**构建时报“拒绝访问”**

- 程序正在运行，先关闭 `fm27-growth-gui.exe` 再构建。
