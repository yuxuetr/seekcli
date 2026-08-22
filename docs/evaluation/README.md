# 评估文档

对 SeekCLI 做外部坐标系对照评估的产出，用来回答「还差什么」而不是「做了什么」。

| 文档 | 内容 |
| --- | --- |
| [scoring-rubric.md](scoring-rubric.md) | 评分口径与缺口分级标准。**先读这个**，否则不同次评估的数字无法比较 |
| [2026-08-harness-gap-analysis.md](2026-08-harness-gap-analysis.md) | 对照 `deepseek-harness` 的完整逐层评估（当前基线） |

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
