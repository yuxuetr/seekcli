//! L8 Loop 层（MVP）：调度器（launchd/cron）驱动的 headless 任务入口。
//! 每个具名任务对应一个硬编码 prompt 模板，复用 App::run_headless——状态的
//! 读取/更新/是否通知/写摘要全部由模型经既有工具完成，本文件不重复实现这些
//! 决策逻辑（AGENT_ARCHITECTURE.md §7）。
//! 后续新增 stocks/learning 等场景：在这里加一个 match 分支 + 一个 prompt
//! 函数即可，不做通用任务定义文件系统（超出本阶段范围）。

use anyhow::{Context, Result};
use chrono::Local;
use std::env;
use std::path::PathBuf;

use crate::{App, Config, Skill};

/// 解析（并创建）任务状态目录。chdir 进这个目录是让模型的 write_file/
/// edit_file 相对路径调用能通过 path_security::ensure_within_cwd 白名单的
/// 关键——和 benchmark.rs 里 testbed 的 chdir 手法完全一致。
fn resolve_dir(config: &Config) -> Result<PathBuf> {
  let dir = match &config.tasks.dir {
    Some(d) => PathBuf::from(d),
    None => {
      let home = env::var("HOME").context("HOME not set")?;
      PathBuf::from(home).join(".seekcli").join("tasks")
    }
  };
  std::fs::create_dir_all(&dir)?;
  std::fs::create_dir_all(dir.join("digest"))?;
  Ok(dir)
}

/// 每个具名任务的固定配方：跑什么 prompt + （可选）预先激活哪个 Skill 提供
/// 方法论/真实数据获取脚本。Skill 按名字从 `~/.seekcli/skills/` 里找——不存在
/// 就报错退出，而不是静默降级成"裸跑"（比如 stocks 任务没有 Skill 就没有真实
/// 行情数据源，跑了也是空转/幻觉）。
fn task_spec(name: &str) -> Result<(String, Option<&'static str>)> {
  match name {
    "reminders" => Ok((reminders_prompt(), None)),
    other => anyhow::bail!("未知任务 '{other}'。当前可用任务：reminders"),
  }
}

/// 按名字加载一个已保存的 Skill（`~/.seekcli/skills/<name>/SKILL.md`）。
fn load_named_skill(app: &App, skill_name: &str) -> Result<Skill> {
  let skills = app.skill_manager.load_skills()?;
  skills
    .into_iter()
    .find(|s| s.name == skill_name)
    .with_context(|| {
      format!(
        "任务需要 skill '{skill_name}'，但未在 ~/.seekcli/skills/ 下找到。\
         请先创建该 skill（含真实数据获取脚本/方法论）再运行此任务。"
      )
    })
}

/// `--run-task <name>` 入口：chdir 进任务目录 → 跑一次 headless agent turn
/// （按需先激活配套 Skill）→ chdir 回来 → 打印结果。
pub async fn run_task(app: &mut App, name: &str) -> Result<()> {
  let (prompt, skill_name) = task_spec(name)?;
  let skill = skill_name.map(|n| load_named_skill(app, n)).transpose()?;

  let dir = resolve_dir(&app.config)?;
  let original_cwd = env::current_dir()?;
  env::set_current_dir(&dir)?;
  let result = app.run_headless(&prompt, skill.as_ref()).await;
  env::set_current_dir(&original_cwd)?;

  let (final_text, calls) = result?;
  println!("[Task:{name}] 完成，{calls} 次模型调用。");
  if !final_text.trim().is_empty() {
    println!("{}", final_text.trim());
  }
  Ok(())
}

/// 交互式 REPL 启动时的横幅提醒：只看"今天的 digest 文件是否存在 + 今天
/// 是否已经提醒过"，不解析 markdown 内容（内容该怎么呈现，模型在生成 digest
/// 时已经决定好了）。一天只提醒一次，不管后台定时任务重写了多少次 digest。
pub fn pending_digest_notice(config: &Config) -> Option<String> {
  let dir = resolve_dir(config).ok()?;
  let today = Local::now().format("%Y-%m-%d").to_string();
  let digest_path = dir.join("digest").join(format!("{today}.md"));
  if !digest_path.exists() {
    return None;
  }

  let marker = dir.join(".banner_last_shown");
  let last_shown = std::fs::read_to_string(&marker).unwrap_or_default();
  if last_shown.trim() == today {
    return None;
  }
  let _ = std::fs::write(&marker, &today);
  Some(format!(
    "[Digest] 今天的摘要已生成：{}",
    digest_path.display()
  ))
}

fn reminders_prompt() -> String {
  let today = Local::now().format("%Y-%m-%d").to_string();
  let now = Local::now().format("%Y-%m-%d %H:%M (%A)").to_string();
  format!(
    r#"你现在是被系统定时任务（launchd/cron，约每 15 分钟一次）无人值守调用的
一次性 headless 运行，不是交互式会话。没有用户在场——绝不要执行需要交互确认
的命令，也不要等待任何输入。

当前本地时间：{now}。

当前工作目录下应该有两个文件：
  - reminders.md — 有时间点的提醒事项，到期需要触发系统通知
  - todos.md      — 无时间点的待办清单，只体现在摘要里，不触发系统通知

# 文件格式
先用 list_dir(".") 看当前目录下实际有哪些文件——**不要直接对可能不存在的文件
调用 read_file**（对不存在的文件调用 read_file 会失败，进而触发系统的失败恢复
机制，反而可能扰乱你紧接着的下一次工具调用）。根据 list_dir 的结果判断：
  - 如果 reminders.md 出现在列表里，才对它调用 read_file；否则视为不存在，
    直接用 write_file 创建一个只有标题、没有条目的空文件。
  - todos.md 同理。

reminders.md，每行一条：
  - [ ] YYYY-MM-DD[ HH:MM] <内容> [notified:YYYY-MM-DD[ HH:MM]]
  - [x] YYYY-MM-DD[ HH:MM] <内容>
末尾可选的 `[notified:...]` 记录"上次通知时对应的到期值"——拿它和这一行自己
的到期值比较：如果不存在，或者不相等（说明用户改过期时间），就需要重新通知。
示例：
  - [ ] 2026-07-17 15:00 牙医预约
  - [ ] 2026-07-20 提交季度报告 [notified:2026-07-19]
  - [x] 2026-07-10 09:00 交电费

todos.md，每行一条（没有具体时间点，到期日期可选）：
  - [ ] 买生日礼物 [due:2026-07-19]
  - [ ] 续费域名
  - [x] 报销上周差旅

# 这次运行要做的事
1. 按上面的规则确认/创建两个文件，并 read_file 已存在的那些。
2. 对 reminders.md 里每一条 `- [ ]` 且到期日期/时间 <= 当前时间的条目：
   a. 如果没有 `[notified:...]` 标记，或者标记的值和这一行自己的到期值不同，
      就触发**恰好一次**通知：
        run_shell: osascript -e 'display notification "<内容>" with title "SeekCLI 提醒"'
      然后用 edit_file 在这一行末尾追加/更新 `[notified:<这次到期值>]`，
      这样 15 分钟后下一次运行就不会重复弹了。
   b. 如果标记的值已经和到期值一致，跳过——已经通知过，不要重复弹窗刷屏。
   c. `- [x]` 的条目、或者还没到期的条目，一律不通知。
   注意：**每条到期提醒只弹一次系统通知**，就算过期后仍未标记完成，也不再
   重复弹窗——过期未完成的项目会体现在下面第 4 步的摘要里，靠摘要持续提醒，
   不靠反复弹窗骚扰。
3. 不要为 todos.md 里的条目触发系统通知——待办事项优先级更低，只出现在摘要里。
4. 用 write_file 生成（覆盖）digest/{today}.md ——一份不超过 20 行、人类能
   一眼扫完的 Markdown 摘要：今天到期的提醒、所有过期未完成的提醒、以及 3 天
   内到期（或没有到期日期，挑几条列出）的待办。这个文件就是用户下次打开
   交互式 REPL 时会看到的内容。
5. 除了追加/更新 `[notified:...]` 标记之外，不要动任何 `- [x]` 行。
6. 最后用**一行纯文本**总结这次做了什么（例如"通知了 2 条提醒，1 条已过期
   未处理，已生成 {today} 的摘要。"），然后停止调用工具。
"#
  )
}
