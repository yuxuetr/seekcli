//! L8 Loop 层：调度器（launchd/cron）驱动的 headless 任务入口。
//!
//! 任务是**声明式**的：一个 `~/.seekcli/tasks/<name>/TASK.md`，YAML frontmatter
//! 加一段 Markdown 正文，正文就是 prompt。此前是 Rust 里的 `match` 分支，加一个
//! 任务要改代码并重新编译——对一个「换套 prompt 再跑一次同一个 Harness」的东西
//! 来说，这个代价说不通。
//!
//! 格式与 `SKILL.md` 同构并共用同一个 frontmatter 切分器：两者都是「YAML 头 +
//! 一段其实是 prompt 的 Markdown 正文」，在 BOM 或 CRLF 上各自漂移毫无意义。
//!
//! 复用 `App::run_headless`——状态的读取 / 更新 / 是否通知 / 写摘要全部由模型经
//! 既有工具完成，本文件不重复实现这些决策逻辑
//! （`docs/architecture/design-principles.md`）。

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

/// 一个任务的声明。
pub struct TaskSpec {
  pub name: String,
  pub description: String,
  /// 预先激活的 Skill——提供方法论 / 真实数据获取脚本。找不到就报错退出，
  /// 而不是静默降级成「裸跑」（比如 stocks 任务没有 Skill 就没有真实行情
  /// 数据源，跑了也是空转 / 幻觉）。
  pub skill: Option<String>,
  /// 建议的调度间隔，只用于生成 plist。SeekCLI 自己不做调度。
  pub interval_hint: Option<String>,
  /// 正文即 prompt。
  pub prompt: String,
}

fn task_file(dir: &std::path::Path, name: &str) -> PathBuf {
  dir.join(name).join("TASK.md")
}

/// 列出所有已定义的任务名。
fn available(dir: &std::path::Path) -> Vec<String> {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return Vec::new();
  };
  let mut out: Vec<String> = entries
    .flatten()
    .filter(|e| e.path().join("TASK.md").is_file())
    .map(|e| e.file_name().to_string_lossy().to_string())
    .collect();
  out.sort();
  out
}

/// 读取任务定义。内置的 reminders 首次运行时自动写出，用户可以直接改。
fn task_spec(dir: &std::path::Path, name: &str) -> Result<TaskSpec> {
  let path = task_file(dir, name);
  if !path.exists() && name == "reminders" {
    write_builtin_reminders(dir)?;
  }
  if !path.exists() {
    let names = available(dir);
    let listing = if names.is_empty() {
      "（还没有任务；在 ~/.seekcli/tasks/<name>/TASK.md 里定义一个）".to_string()
    } else {
      names.join(", ")
    };
    anyhow::bail!("未知任务 '{name}'。当前可用任务：{listing}");
  }

  let content =
    std::fs::read_to_string(&path).with_context(|| format!("读取 {} 失败", path.display()))?;
  let (yaml, body) = crate::skills::split_frontmatter(&content, "TASK.md")
    .with_context(|| format!("解析 {} 失败", path.display()))?;
  if body.trim().is_empty() {
    anyhow::bail!("{} 的正文为空——正文就是要跑的 prompt", path.display());
  }
  Ok(TaskSpec {
    name: crate::skills::frontmatter_scalar(&yaml, "name").unwrap_or_else(|| name.to_string()),
    description: crate::skills::frontmatter_scalar(&yaml, "description").unwrap_or_default(),
    skill: crate::skills::frontmatter_scalar(&yaml, "skill"),
    interval_hint: crate::skills::frontmatter_scalar(&yaml, "interval_hint"),
    prompt: body,
  })
}

/// 把内置的 reminders 任务写成文件，让它和用户自定义任务走完全相同的路径。
fn write_builtin_reminders(dir: &std::path::Path) -> Result<()> {
  let task_dir = dir.join("reminders");
  std::fs::create_dir_all(&task_dir)?;
  let content = format!(
    "---\n\
     name: reminders\n\
     description: 检查到期提醒与待办，必要时发系统通知，并生成当日摘要\n\
     skill: null\n\
     interval_hint: 15m\n\
     ---\n\
     \n\
     {}\n",
    reminders_prompt()
  );
  std::fs::write(task_dir.join("TASK.md"), content)?;
  eprintln!(
    "[Task] 已写出内置任务定义 {}（可直接编辑）",
    task_file(dir, "reminders").display()
  );
  Ok(())
}

/// 列出已定义的任务，供 `seekcli task list`。
pub fn list(config: &Config) -> Result<String> {
  let dir = resolve_dir(config)?;
  // 让内置任务在首次 list 时也出现，而不是只在首次 run 时才存在。
  if !task_file(&dir, "reminders").exists() {
    write_builtin_reminders(&dir)?;
  }
  let names = available(&dir);
  if names.is_empty() {
    return Ok(format!(
      "还没有任务。新建一个：{}/<name>/TASK.md",
      dir.display()
    ));
  }
  let mut out = String::new();
  for name in names {
    match task_spec(&dir, &name) {
      Ok(spec) => out.push_str(&format!(
        "{:<12} {:<8} {}\n",
        spec.name,
        spec.interval_hint.as_deref().unwrap_or("-"),
        spec.description
      )),
      // 一个坏掉的定义不该让整张表都读不出来。
      Err(e) => out.push_str(&format!("{:<12} {:<8} [无法解析: {}]\n", name, "-", e)),
    }
  }
  Ok(out)
}

/// 依据 `interval_hint` 生成 launchd plist，打到 stdout。
///
/// 只打印、不安装：往用户的 LaunchAgents 里塞东西应当是显式动作。
pub fn render_plist(config: &Config, name: &str) -> Result<String> {
  let dir = resolve_dir(config)?;
  let spec = task_spec(&dir, name)?;
  let seconds = spec
    .interval_hint
    .as_deref()
    .and_then(parse_interval)
    .unwrap_or(900);
  let exe = std::env::current_exe()
    .map(|p| p.display().to_string())
    .unwrap_or_else(|_| "seekcli".to_string());
  Ok(format!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
     <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
     \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
     <plist version=\"1.0\">\n\
     <dict>\n\
     \x20 <key>Label</key><string>com.seekcli.{name}</string>\n\
     \x20 <key>ProgramArguments</key>\n\
     \x20 <array>\n\
     \x20   <string>{exe}</string>\n\
     \x20   <string>--run-task</string>\n\
     \x20   <string>{name}</string>\n\
     \x20 </array>\n\
     \x20 <key>StartInterval</key><integer>{seconds}</integer>\n\
     \x20 <key>EnvironmentVariables</key>\n\
     \x20 <dict>\n\
     \x20   <!-- launchd does not read your shell profile: set the key here. -->\n\
     \x20   <key>DEEPSEEK_API_KEY</key><string>REPLACE_ME</string>\n\
     \x20 </dict>\n\
     </dict>\n\
     </plist>\n"
  ))
}

/// `15m` / `2h` / `90s` / 裸秒数 → 秒。
fn parse_interval(hint: &str) -> Option<u64> {
  let hint = hint.trim();
  let (digits, mult) = match hint.chars().last()? {
    's' => (&hint[..hint.len() - 1], 1),
    'm' => (&hint[..hint.len() - 1], 60),
    'h' => (&hint[..hint.len() - 1], 3600),
    'd' => (&hint[..hint.len() - 1], 86_400),
    _ => (hint, 1),
  };
  digits.trim().parse::<u64>().ok().map(|n| n * mult)
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
  let dir = resolve_dir(&app.config)?;
  let spec = task_spec(&dir, name)?;
  let skill = spec
    .skill
    .as_deref()
    .map(|n| load_named_skill(app, n))
    .transpose()?;
  let prompt = spec.prompt;

  let original_cwd = env::current_dir()?;
  env::set_current_dir(&dir)?;
  let result = app.run_headless(&prompt, skill.as_ref()).await;
  env::set_current_dir(&original_cwd)?;

  let outcome = result?;
  let (final_text, calls) = (outcome.text, outcome.llm_calls);
  eprintln!("[Task:{name}] 完成，{calls} 次模型调用。");
  if !final_text.trim().is_empty() {
    eprintln!("{}", final_text.trim());
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

#[cfg(test)]
mod tests {
  use super::*;

  fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("seekcli-tasks-{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
  }

  fn write_task(dir: &std::path::Path, name: &str, content: &str) {
    let d = dir.join(name);
    let _ = std::fs::create_dir_all(&d);
    let _ = std::fs::write(d.join("TASK.md"), content);
  }

  #[test]
  fn a_task_is_defined_by_a_file_not_by_rust_code() {
    let dir = scratch("declarative");
    write_task(
      &dir,
      "standup",
      "---\nname: standup\ndescription: 汇总昨天做了什么\nskill: notes\ninterval_hint: 1d\n---\n\n看一眼 notes.md 并写摘要。\n",
    );
    let spec = match task_spec(&dir, "standup") {
      Ok(s) => s,
      Err(e) => panic!("parse failed: {}", e),
    };
    assert_eq!(spec.name, "standup");
    assert_eq!(spec.skill.as_deref(), Some("notes"));
    assert_eq!(spec.interval_hint.as_deref(), Some("1d"));
    assert!(spec.prompt.contains("notes.md"), "body is the prompt");
  }

  #[test]
  fn skill_null_reads_the_same_as_omitting_it() {
    let dir = scratch("nullskill");
    write_task(&dir, "a", "---\nname: a\nskill: null\n---\n\nbody\n");
    write_task(&dir, "b", "---\nname: b\n---\n\nbody\n");
    let a = task_spec(&dir, "a").ok().and_then(|s| s.skill);
    let b = task_spec(&dir, "b").ok().and_then(|s| s.skill);
    assert_eq!(a, None);
    assert_eq!(b, None);
  }

  #[test]
  fn an_empty_body_is_rejected_because_the_body_is_the_prompt() {
    let dir = scratch("emptybody");
    write_task(&dir, "hollow", "---\nname: hollow\n---\n\n\n");
    assert!(task_spec(&dir, "hollow").is_err());
  }

  #[test]
  fn an_unknown_task_lists_what_is_available() {
    let dir = scratch("unknown");
    write_task(&dir, "alpha", "---\nname: alpha\n---\n\nbody\n");
    let err = match task_spec(&dir, "nope") {
      Ok(_) => panic!("expected an error"),
      Err(e) => format!("{:#}", e),
    };
    assert!(err.contains("alpha"), "got: {}", err);
  }

  #[test]
  fn the_builtin_task_materialises_as_an_editable_file() {
    // It must go through the same path as a user-defined task, or the two
    // would drift and only one of them would be exercised.
    let dir = scratch("builtin");
    let spec = match task_spec(&dir, "reminders") {
      Ok(s) => s,
      Err(e) => panic!("builtin should self-install: {}", e),
    };
    assert!(task_file(&dir, "reminders").exists());
    assert_eq!(spec.interval_hint.as_deref(), Some("15m"));
    assert!(!spec.prompt.trim().is_empty());
  }

  #[test]
  fn interval_hints_parse_in_the_units_people_write() {
    assert_eq!(parse_interval("90s"), Some(90));
    assert_eq!(parse_interval("15m"), Some(900));
    assert_eq!(parse_interval("2h"), Some(7200));
    assert_eq!(parse_interval("1d"), Some(86_400));
    assert_eq!(parse_interval("600"), Some(600));
    assert_eq!(parse_interval("soon"), None);
  }

  #[test]
  fn a_broken_definition_does_not_hide_the_other_tasks() {
    let dir = scratch("broken");
    write_task(
      &dir,
      "good",
      "---\nname: good\ndescription: fine\n---\n\nbody\n",
    );
    write_task(&dir, "bad", "no frontmatter at all\n");
    let mut listing = String::new();
    for name in available(&dir) {
      match task_spec(&dir, &name) {
        Ok(s) => listing.push_str(&s.name),
        Err(_) => listing.push_str(&format!("{}:broken", name)),
      }
    }
    assert!(listing.contains("good"), "got: {}", listing);
    assert!(listing.contains("bad:broken"), "got: {}", listing);
  }
}
