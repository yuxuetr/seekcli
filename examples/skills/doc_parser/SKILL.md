---
name: doc_parser
description: 通过 MinerU 高保真解析 PDF / Docx / PPTX / Xlsx / 图像 为 Markdown（含 OCR）
version: "2"
---

# Document Parser Skill

DeepSeek V4 不能直接读 PDF / 图片等二进制内容。激活本 skill 后，按
以下流程：

## 工作流 A：解析文档（PDF / Docx / PPTX / Xlsx）

1. 获取文件路径（用户给的，或 list_dir / run_shell 找到）
2. `run_shell: bash <scripts_dir>/mineru_parse.sh <file_path>`
   返回**生成的 Markdown 文件路径**（不是内容本身，避免 stdout 过大）
3. `read_file` 读这个路径
   - 50KB 自动截断；大文档时用 run_shell + grep 定位关键段
4. 基于 Markdown 完成用户的实际任务

## 工作流 B：OCR 剪贴板里的图片

1. `run_shell: bash <scripts_dir>/clip_to_png.sh`
   把 macOS 剪贴板里的图保存到 `/tmp/seekcli_clip.png`
2. `run_shell: bash <scripts_dir>/mineru_parse.sh /tmp/seekcli_clip.png`
   MinerU 内部走 OCR (`is_ocr: true`)，返回 Markdown 路径
3. read_file → 把识别出的文字 / 表格 / 公式呈现给用户

## 工作流 C：OCR 给定路径的图片

直接 `mineru_parse.sh <image_path>`，跳过 clip_to_png 步骤。

## 注意事项

- 需要 `MINERU_API_KEY` 环境变量
- MinerU 是远程异步 API：典型耗时 5-30 秒，长文档可能更久
  （脚本最多等 120 秒）
- 支持格式：PDF / Docx / PPTX / Xlsx / PNG / JPG / WebP / BMP
- 输出路径形如 `/tmp/seekcli_mineru_<timestamp>.md`，重启后系统自动清

## 失败处理规范（**重要**）

如果 mineru_parse.sh 返回错误（超时 / API 失败 / 网络问题），你**必须**
按以下顺序处理，**不要静默装新工具或换技术栈**：

1. 把 stderr 里的错误信息**原样**报告给用户
2. 询问用户希望怎么处理：
   - 重试一次？（偶尔的网络抖动）
   - 换用 vision skill 描述图像（如果输入是图）？
   - 用户自己提供另一份解析后的内容？
3. **绝对禁止**：
   - 未经用户同意 `pip install` / `brew install` / `cargo install` 等装包操作
   - 改用其他 PDF 解析库（PyMuPDF / pdfplumber 等）
   - 把 PDF 转图片再走 VLM 这种弯路

doc_parser skill 的定义就是"用 MinerU"。MinerU 不行就停下问用户，不要
自创替代方案 —— 那会绕过用户选择 MinerU 时的设计意图（高保真表格 /
公式 / 版面重建）。

## 与 vision skill 的分工

- **vision** (StepFun VLM)：图像**视觉理解**（含 OCR，但侧重描述与
  语义）— 适合"这张图什么内容"、"图里发生了什么"
- **doc_parser** (MinerU)：**文本提取**为主（OCR 高保真，含表格 /
  公式重建）— 适合"提取图里的所有文字"、"解析 PDF / 截图为
  Markdown"

如果用户说"OCR"、"提取文字"、"识别表格"，优先用 doc_parser。
如果用户说"描述图片"、"图里有什么"，优先用 vision。

## 工作示例

用户："OCR 一下剪贴板里这张图"

你应该：
1. clip_to_png.sh → /tmp/seekcli_clip.png
2. mineru_parse.sh /tmp/seekcli_clip.png → /tmp/seekcli_mineru_xxx.md
3. read_file 拿到 Markdown 内容
4. 把识别出的文字 / 表格还原给用户，注明它来自 MinerU OCR
