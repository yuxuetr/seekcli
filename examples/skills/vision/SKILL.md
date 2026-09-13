---
name: vision
description: "[已过时] 曾用外部 VLM (StepFun) 补图像理解；deepseek-flash 现在自己能看图"
version: "1"
---

# Vision Skill（已过时）

> ⚠️ **这个 skill 的前提已经不成立了。**
>
> 它原本的开篇是「DeepSeek V4 是纯文本模型，本身不能"看"图像」。
> 2026-09-13 实测：`deepseek-flash` **自己就能看图**——一张 64×64 左蓝右黄的
> PNG，它答 `Blue on the left, yellow on the right.`
> 验证方法见 [L0 §4.5.2](../../../docs/architecture/L0-llm-substrate.md)。
>
> **所以激活它只会让 agent 白跑一趟外部 VLM**，多花一次 StepFun 调用、
> 多等一个来回，换来一段比原图信息量更少的文字描述。
>
> 让图像真的进入请求需要多段内容支持（缺口 L4-8，架构级，见
> [路线图阶段四十一](../../../TODOs.md)）。**在那之前图像还进不了上下文**，
> 所以这个 skill 仍是当下唯一可用的退路——保留它是为此，不是因为它还正确。
>
> 阶段四十一落地后应当删除本目录。

激活本 skill 后，遇到需要理解图像的任务时，按以下流程执行：

## 工作流

### 来源 1：剪贴板（最常见）
```
1. run_shell: bash <scripts_dir>/clip_to_png.sh
   → 把 macOS 剪贴板里的图保存到 /tmp/seekcli_clip.png 并打印路径
2. run_shell: bash <scripts_dir>/vlm_describe.sh /tmp/seekcli_clip.png
   → 返回中文详细描述
3. 把描述当作"视觉证据"继续完成用户的实际任务
```

### 来源 2：用户给的文件路径
```
1. run_shell: bash <scripts_dir>/vlm_describe.sh <user_path>
2. 同上
```

## 注意事项

- 需要 `STEP_API_KEY` 环境变量。脚本会自检；未设置时返回明确的错误信息
- VLM 描述偶有误差。如果用户对某细节存疑，可以**追问 vlm_describe.sh
  时附带具体问题**（脚本支持第二个参数作为定向提问）
- 不要尝试把 base64 图像数据塞进对话 —— 那不是文本。只用 VLM 文本输出
- 大图建议先 `sips -Z 1280` 压缩再描述（节省 token，加速调用）

## 工作示例

用户："帮我看看剪贴板里这张图什么内容"

你应该：
1. 调 clip_to_png.sh
2. 调 vlm_describe.sh /tmp/seekcli_clip.png
3. 把 VLM 返回的描述整理后告诉用户，并主动提出"需要进一步分析哪个细节吗"
