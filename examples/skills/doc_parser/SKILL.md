---
name: doc_parser
description: 用 MinerU 把 PDF / Docx / PPTX / Xlsx / 图片逐字逐格转成 Markdown。需要精确提取表格、公式、整页文字，或要解析非图片文档时用；只是看懂一张图不需要它——你自己能看图
version: "3"
---

# Document Parser Skill

你自己能看图（`/paste` 贴进来的、`read_image` 读到的都会直接给你看）。
本 skill 不是给你补视觉，而是补**逐字逐格的精确转录**——这是看图最容易出错的地方。

## 什么时候用 MinerU，什么时候自己看

| 任务 | 做法 |
| --- | --- |
| 「这张图是什么」「报错在哪」「趋势如何」 | **自己看**，不调 MinerU |
| 提取表格、公式、整页文字；图里的数字之后要拿去计算或引用 | **MinerU** |
| PDF / Docx / PPTX / Xlsx（你没法直接读） | **MinerU** |

拿不准时按后果判断：**读错一个数字会不会导致错误结论**？会，就走 MinerU。
密集表格是看图最容易错格、编出数字而且看起来很可信的地方。

## 工作流

1. **拿到文件路径**
   - 用户贴的图：消息里有一行 `[attached image saved at <path>]`，用这个路径
   - 文档：用户给的路径，或用 `list_dir` / `grep` 找到
2. `run_shell: bash <scripts_dir>/mineru_parse.sh <path>`
   stdout 只返回**生成的 Markdown 文件路径**（内容不进 stdout，免得撑爆上下文）
3. `read_file` 读这个路径。大文档会只给头尾预览，用 `grep` 在原文件里定位需要的段落
4. **对照核对**（输入是图片时）：你同时有原图和 MinerU 的结果，核一遍表头、
   行列对齐、数字。发现不一致就**指出来**，不要悄悄改成你看到的版本——
   两边哪个对需要说出依据
5. 交付结果，注明表格 / 文字来自 MinerU 提取

## 注意事项

- 需要 `MINERU_API_KEY` 环境变量，以及 `jq`、`unzip`
- MinerU 是远程异步 API：文件会上传到 mineru.net，典型耗时 5–30 秒，脚本最多等 120 秒
- 单文件上限 200MB
- 输出在 `/tmp/seekcli_mineru_<timestamp>.md`

## 失败处理（**重要**）

`mineru_parse.sh` 报错（超时 / API 失败 / 网络 / key 失效）时：

1. 把 stderr 的错误信息**原样**告诉用户
2. 问用户怎么办：重试一次？还是接受由你直接看图给出结果——
   并**明说**这样表格和数字的精度不如 MinerU
3. **禁止**：
   - 未经同意 `pip install` / `brew install` / `cargo install` 装包
   - 换用其他解析库（PyMuPDF / pdfplumber 等）自创替代方案
   - 在用户不知情的情况下改成自己看图，却把结果当成精确提取交付

MinerU 是用户为高保真表格 / 公式 / 版面重建选定的工具。它不可用时停下来问，
而不是悄悄换掉——换掉的那一刻，「这些数字是精确提取的」就不再成立了。
