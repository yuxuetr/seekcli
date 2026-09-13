# 评估文档

对 SeekCLI 做外部坐标系对照评估的产出，用来回答「还差什么」而不是「做了什么」。

| 文档 | 内容 |
| --- | --- |
| [scoring-rubric.md](scoring-rubric.md) | 评分口径与缺口分级标准。**先读这个**，否则不同次评估的数字无法比较 |
| [2026-08-harness-gap-analysis.md](2026-08-harness-gap-analysis.md) | 首评：对照 `deepseek-harness` 的完整逐层评估（缺口编号在此定义） |
| [2026-08-24-reassessment.md](2026-08-24-reassessment.md) | **复评**：推进完阶段二十 ~ 三十三后的复盘（当前基线） |
| [2026-09-12-self-evolution-baseline.md](2026-09-12-self-evolution-baseline.md) | **三评**：换用 D 口径（自进化地基）的基线，新增缺口 L1-6 / L4-7 / L5-6 / L7-6 / L7-7 / L7-8 |
| [2026-09-13-substrate-shift.md](2026-09-13-substrate-shift.md) | **基底变更**：`deepseek-flash` 具备视觉，项目三处写死的前提由真变假。无坐标系——这不是对照外部清单，新增缺口 L0-5 / L2-9 / L4-8 / L6-6 |

## 与其它文档的关系

```
docs/evaluation/    发现缺口，给优先级        ← 你在这里
        │
        ▼
docs/architecture/  为每个缺口设计补全方案
        │
        ▼
TODOs.md            把方案拆成可执行、可验收的阶段
```

评估只负责「指出缺什么、值不值得补」；**怎么补**在 `docs/architecture/` 的对应层文档里，
**什么时候补**在根目录 `TODOs.md` 里。
