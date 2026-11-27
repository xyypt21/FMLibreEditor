# 阵容窗口

`fm27-squad-gui.exe` 是独立的阵容窗口：只读游戏，显示 ACA/APA、阵型缺口和可出售球员。它和增长窗口共用同一份设置文件，但规则层是 `crates/fm27-growth-model`，两个窗口不会各自维护一套算法。

## ACA 与 APA

```text
ca_after_years(ca, years) = round(ca + monthly_gain × 12 × years)

年龄未知或 age >= max_age：
    APA = CA
    ACA = CA

否则：
    APA = min(PA, ca_after_years(CA, max_age - age), ca_after_years(CA, apa_shortfall_years))
    ACA = max(CA, min(ca_after_years(CA, aca_growth_years), cap(age, PA), APA))
```

- `cap` 与增长窗口相同，是阶梯上限，不做线性插值；
- 默认 `monthly_gain = 2`、`max_age = 23`、`apa_shortfall_years = 2.0`、`aca_growth_years = 0.5`；
- 这两个 `*_years` 只影响预测，不改变实际写入。

## 阵型与深度配额

阵型在窗口顶部直接编辑，格式是 `位置 首发人数`，用逗号、分号或换行分隔，也可以写 `位置=人数`：

```text
GK 1, WBL 1, WBR 1, DC 3, MC 2, AML 1, AMR 1, ST 1
```

- 首发人数合计必须是 11；
- 位置使用 FM 的 15 个位置代码：`GK SW DL DC DR DM ML MC MR AML AMC AMR ST WBL WBR`；
- 守门员配额固定为 2；
- 其他位置配额 = 首发人数 × 2 + 2，所以 1 人需要 4 个，2 人需要 6 个，3 人需要 8 个。

## 选人

1. 排除“租借在外”的球员，租借到本队的球员保留；
2. 能踢某位置的条件是游戏给该位置的评分 ≥ 15；
3. 按 ACA 从高到低排序，ACA 相同按 APA 从高到低；
4. 从最好的球员开始，只要他能踢的位置还有空位就放进去；一个球员可以同时占多个位置；
5. 没有填入任何位置的球员进入“落选”列表。

## 地板、薄弱与覆盖

```text
floor    = 入选球员 APA 从低到高排序后，第 ceil(20% × 人数) 个的值
weak     = 入选且 APA <= floor 的球员
cover    = 落选、能踢该位置、APA > floor 的球员，按 APA 从高到低
missing  = max(weak 数 - cover 数, 0)
```

- `floor` 相同值的球员都算薄弱；
- 一个薄弱球员需要一个 cover，两个需要两个；
- 没有 cover 时显示 `NEEDS DEPTH xN`，有 cover 时显示 `covered by 名字 (APA n)`。

## 出售

落选球员满足以下任一条件时列出：

```text
age > max_age
或
age > max_age - apa_shortfall_years 且 APA <= floor
```

年龄未知的球员不列出。窗口按年龄从大到小排列。

## 窗口与刷新

```text
顶部     Pause / Resume | Refresh now | Squad rules | 状态
阵型     Formation 文本框 + Reset
中间     Depth 配额、Positions 表格、Needs depth 汇总
底部     Sell candidates 表格
```

- 自动连接，每 3 秒重试；
- 每 30 秒读取一次；
- `Pause` 停止自动读取，`Refresh now` 立即读取；
- 读取失败时状态栏显示错误，保留上一次结果；
- 关闭窗口时保存阵型。

## 设置

与增长窗口共用：

```text
%APPDATA%\fm27-growth\settings.json
```

阵容窗口只修改 `formation`，退出时通过“读回最新文件再改自己字段”的方式保存，因此不会覆盖增长窗口的值。增长窗口同理，只保存自己的数值字段。
