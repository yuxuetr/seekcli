---
name: doc_parser
description: 通过 MinerU 高保真解析 PDF / Docx / PPTX / Xlsx / 图像 为 Markdown
version: "1"
---

# Document Parser Skill

DeepSeek V4 不能直接读 PDF 等二进制文档。激活本 skill 后，遇到这类
任务时按以下流程：

## 工作流

1. **获取文件路径**：用户给的，或用 list_dir / run_shell 找到
2. **调用 MinerU 解析**：
   ```
   run_shell: bash <scripts_dir>/mineru_parse.sh <file_path>
   ```
   返回值是**生成的 Markdown 文件路径**（不是内容本身，避免 stdout 巨长）
3. **读取 Markdown**：用 read_file 读这个路径
   - 50KB 自动截断；大文档时用 run_shell + grep 定位关键段
4. **基于 Markdown 继续完成用户的实际任务**

## 注意事项

- 需要 `MINERU_API_KEY` 环境变量
- MinerU 是远程异步 API，单文件解析典型耗时 5-30 秒，长文档可能更久。
  脚本内置 60 次轮询（每次 2 秒，总共最多等 120 秒）
- 支持格式：PDF / Docx / PPTX / Xlsx / PNG / JPG / WebP / BMP
- 生成的 Markdown 路径形如 `/tmp/seekcli_mineru_<timestamp>.md`，
  调用结束后不会自动清理（方便用户事后查看），系统重启时自然消失

## 工作示例

用户："帮我总结一下 ~/Downloads/paper.pdf 的核心观点"

你应该：
1. run_shell: `bash <scripts_dir>/mineru_parse.sh ~/Downloads/paper.pdf`
   得到 `/tmp/seekcli_mineru_xxx.md`
2. read_file `/tmp/seekcli_mineru_xxx.md`
   （若被 50KB 截断，用 grep "abstract\|conclusion" 定位）
3. 总结核心观点，引用文中具体段落
