---
name: finance
description: 用 Tushare 取 A 股 / 指数 / 基金 / 财务 / 资金流 / 宏观等数据，做行情、估值、财务、对比与图表分析。用户问股票、行情、财报、估值、板块、宏观指标时用。只读研究，不下单
version: "1"
---

# Finance Skill（Tushare）

把「看看这只票最近怎么样」「比较一下 A 和 B 的估值」这类问题，变成
**取数 → 计算 → 出图 → 结论**。数据来自 Tushare Pro，落盘为 parquet。

## 研究模式（硬规则）

**只读取和计算，不下单、不转账、不调用任何交易接口。**用户要求交易时，说明本 skill
不做，给出分析结论由用户自己决定。

## 环境

不用装任何东西：脚本用 `uv run --script` 运行，依赖写在脚本头部，首次运行自动建环境。

```
uv run --script <scripts_dir>/ts.py apis <关键词>      # 查接口名（如 日线 / 财务指标 / 资金流）
uv run --script <scripts_dir>/ts.py doc <接口名>       # 看入参、出参、积分与频次限制
uv run --script <scripts_dir>/ts.py fetch <接口名> k=v ... [--fields a,b]
```

- 需要 `TUSHARE_TOKEN` 环境变量。用户约 2000 积分：常规日线、财务、指数够用；
  `rt_*` 实时接口和部分港美股接口可能另需权限——以 `fetch` 的报错为准，不要猜
- `fetch` 的 stdout 只有：parquet 路径、行数、取数时间、列名、日期范围。**数据不进上下文**
- 同一查询会覆盖同一个文件；`~/.seekcli/data/tushare/<接口名>/*.parquet` 就是本地库

## 工作流

1. **定标的与口径**：不确定代码就 `fetch stock_basic --fields ts_code,name,industry,list_date`
   再在数据里查名字；没给时间范围时默认近一年，并说出来
2. **不确定接口就先查**：`apis` → `doc`，按文档的参数名和日期格式（YYYYMMDD）调用
3. **取数**：`fetch`。返回 `EMPTY` 时先查参数、是不是交易日，**不要据此断言「没有数据」**
4. **计算**：写分析脚本放在工作目录，用
   `uv run --with pandas --with pyarrow --with duckdb --with matplotlib python <脚本>` 运行。
   多文件可直接 `duckdb.sql("select ... from read_parquet('~/.seekcli/data/tushare/daily/*.parquet')")`，
   注意不同查询的日期区间可能重叠，需要时 `select distinct`
5. **出图**（需要时）：matplotlib 存 PNG 到工作目录，中文字体设
   `plt.rcParams['font.sans-serif'] = ['Hiragino Sans GB', 'Arial Unicode MS']`；
   存完用 `read_image` 看一眼自己的图，坐标、标题、图例对了再交付
6. **交付**：按下面的三分法写

## 交付三分法

结论里的每一句都要能归到下面一类，并且写法不同：

| 类别 | 必须带上 |
| --- | --- |
| **事实**（取到的数据） | 来源接口、数据日期、单位与口径（如 `amount` 单位千元、`daily` 未复权） |
| **计算结果** | 输入是哪份数据、用了什么方法（如「近 20 个交易日收盘价的年化波动率」） |
| **判断**（贵不贵、强不强） | 依据的假设，以及什么情况下这个判断会错 |

不要把判断写成事实。「估值偏高」是判断，「PE(TTM) 为 35.2，近五年 80% 分位」是事实 + 计算。

## 时效性（每次都要说清）

- **数据截至哪天**：用 `fetch` 输出的日期范围，不要用「最新」「当前」糊过去
- **实时还是收盘**：`daily` 等日线是收盘后入库（交易日约 15–16 点）；盘中问「现在多少」
  时，日线只能给上一个交易日的收盘，必须明说
- **公告日 ≠ 报告期**：财务数据区分 `ann_date`（发布）和 `end_date`（报告期）
- **财务可能被重述**：同一报告期可能有多条记录，留意 `update_flag`，取最新那条并说明

## 失败处理

- `fetch` 报错（权限、积分、参数）：把错误原文告诉用户，说明影响了哪部分结论
- 接口不可用时可以换一个 Tushare 接口；或用网页搜索补，但**必须标出来源不同**
- **禁止**：未经同意 `pip install` 或换用其他数据库（akshare 等）；
  把搜索到的数字当成 Tushare 数据交付；在报错后编造数字
