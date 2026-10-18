# Changelog

All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

---
## [0.3.0](https://github.com/yuxuetr/seekcli/compare/v0.2.0..v0.3.0) - 2026-10-18

### Bug Fixes

- **(approval)** 审批提示一次只允许一个 - ([324a83b](https://github.com/yuxuetr/seekcli/commit/324a83bc1baca5a5c35f2b508f8ce4306e3494c7)) - yuxuetr
- **(audit)** 成功的写入也进审计日志 - ([5cea72f](https://github.com/yuxuetr/seekcli/commit/5cea72f924885b7164bb5bbf2e3a58859e2304f5)) - yuxuetr
- **(audit)** 测试不再写真实审计日志，并发追加不再交织 - ([ff70dd8](https://github.com/yuxuetr/seekcli/commit/ff70dd85441360bb0ee3ef08448a13c885fc5341)) - yuxuetr
- **(changelog)** 去掉跳过一切中文提交的模板规则 - ([1b07671](https://github.com/yuxuetr/seekcli/commit/1b0767105ccd796527e4f40cf96fbea2bb214b90)) - yuxuetr
- **(config)** 项目层 .seekcli.toml 须经用户信任才生效 - ([ade0a8b](https://github.com/yuxuetr/seekcli/commit/ade0a8bcadd7f5b0a82afa12d9b66aff2832cb73)) - yuxuetr
- **(deps)** async-trait 升到 0.1.92，修复新版 clippy 的 double_must_use - ([7896f06](https://github.com/yuxuetr/seekcli/commit/7896f06b8f4e2b53b1dbc18db39b6f9d541a2a10)) - yuxuetr
- **(docs)** 推翻我自己的「run_shell 没有超时」，并记下它牵出的真缺口 - ([3a2500e](https://github.com/yuxuetr/seekcli/commit/3a2500e02e04f88924e807adf8d7c90933856bae)) - yuxuetr
- **(engine)** 中断传播进子代理，只有顶层消费标志位 - ([9d9641d](https://github.com/yuxuetr/seekcli/commit/9d9641da2b49fa304393f58cffcc3f43445433ac)) - yuxuetr
- **(engine)** 并行委派分支漏走了回合尾部 - ([817a264](https://github.com/yuxuetr/seekcli/commit/817a2647a89557186e90909710518741b07f0ab9)) - yuxuetr
- **(engine)** 并发子代理各记各的耗时 - ([a5b1c38](https://github.com/yuxuetr/seekcli/commit/a5b1c38090f0cd16f27a36f008fff22f7fdab612)) - yuxuetr
- **(engine)** Two-Stage 的计划要进事件日志 - ([020ccea](https://github.com/yuxuetr/seekcli/commit/020ccea2d8ca2a064d065691017b18a0cf54512d)) - yuxuetr
- **(engine)** 撞到迭代上限的子代理不再一无所获 - ([5fd7a73](https://github.com/yuxuetr/seekcli/commit/5fd7a7359328564134e53ac2ec50876e807183d7)) - yuxuetr
- **(engine)** 被输出上限截断的回复不再记为 Completed - ([7bad472](https://github.com/yuxuetr/seekcli/commit/7bad4726914e4fe3d85a7a2b0c5cc6075c06bf27)) - yuxuetr
- **(engine)** 被截断的工具调用不再执行，并告知模型拆分 - ([083d084](https://github.com/yuxuetr/seekcli/commit/083d08455aa9596db26734688d8b5a5c4c505ea0)) - yuxuetr
- **(engine)** 验收命令被 Ctrl-C 打断时运行记为 Interrupted - ([fa6dc37](https://github.com/yuxuetr/seekcli/commit/fa6dc37ad8501b5a3503424a05dc8d5a0383feb2)) - yuxuetr
- **(engine)** 验收命令的输出也回显给用户 - ([4b8a71e](https://github.com/yuxuetr/seekcli/commit/4b8a71ef1b7610e42dba9641c96cda09b491414a)) - yuxuetr
- **(fs)** 文件替换改为原子，edit_file 按路径串行 - ([5ac451a](https://github.com/yuxuetr/seekcli/commit/5ac451a1df1afb89300ddb970c8aeeccae4eefae)) - yuxuetr
- **(policy)** 重定向到 /dev/null 不再被判为越界写 - ([32f8108](https://github.com/yuxuetr/seekcli/commit/32f8108771081132eba6d7802607f291309477d8)) - yuxuetr
- **(policy)** cd 之后的相对路径不再绕过越界写审批 - ([7cc8089](https://github.com/yuxuetr/seekcli/commit/7cc8089f257abca086cf4398d0addc8a2f5bea9a)) - yuxuetr
- **(render)** 半行内容会排到工具调用通知的后面 - ([3e5f8b3](https://github.com/yuxuetr/seekcli/commit/3e5f8b30cf52f1f133ed1a06e991cbd9775dfd02)) - yuxuetr
- **(render)** [Usage] 会插进答案中间 —— 上一轮我只修了三处里的两处 - ([2163861](https://github.com/yuxuetr/seekcli/commit/2163861753a5572dedb810fee8a637428866ceca)) - yuxuetr
- **(repl)** slash 命令只保留一份清单，补全不再漏 5 条 - ([b471256](https://github.com/yuxuetr/seekcli/commit/b471256e9644a17e78df9f5d632b545dd2996932)) - yuxuetr
- **(scripts)** 批次分组的重估条件改为按时间判定 - ([a3debb3](https://github.com/yuxuetr/seekcli/commit/a3debb3f860611cf2c7e349eb75dc185c5acc15b)) - yuxuetr
- **(shell)** 取消与 job_kill 杀整个进程组，不再留下孤儿 - ([397e25f](https://github.com/yuxuetr/seekcli/commit/397e25f831d3069dff4ce7064626066546903f0f)) - yuxuetr
- **(shell)** 每次 run_shell 都白付 800ms 的 spinner 延迟 - ([fcca4ef](https://github.com/yuxuetr/seekcli/commit/fcca4ef9b379a530c065f59e0bd7d5c3c379949a)) - yuxuetr
- **(shell)** 子进程的 stdin 给 /dev/null，否则交互式命令挂死 agent - ([b432029](https://github.com/yuxuetr/seekcli/commit/b432029e6f2ba7152e6188075a6862b5367ba9d7)) - yuxuetr
- **(shell)** 进程组 kill 改用 POSIX 写法；stdin 探针在 Linux 上跟随符号链接 - ([4482f57](https://github.com/yuxuetr/seekcli/commit/4482f57e8a23953dc8d0470f950ea07d33263f01)) - yuxuetr
- **(shell,approval)** 超时不再漏掉清理；Ctrl-C 在审批提示上终于有反应 - ([0d5b4fe](https://github.com/yuxuetr/seekcli/commit/0d5b4fefa45778c159c22aa08e8ac906485ffbf3)) - yuxuetr
- **(skills)** references/ 清单递归并显式截断 - ([26976e8](https://github.com/yuxuetr/seekcli/commit/26976e83434362d898e10576e642ed53e38efc97)) - yuxuetr
- **(subagents)** 只有标了 parallel_safe 的模板才扇出 - ([49895ef](https://github.com/yuxuetr/seekcli/commit/49895ef74922fc7a99ce4fdaea92384b69458c8d)) - yuxuetr
- **(tasks)** 本地时间在每次运行时注入，不再冻结在 TASK.md 里 - ([970e042](https://github.com/yuxuetr/seekcli/commit/970e0422e263bbb5aafbaf7d1ca6bbb1faa215a3)) - yuxuetr
- **(test)** 测试不再往真实会话目录写东西 - ([f605d22](https://github.com/yuxuetr/seekcli/commit/f605d22385737dd3bf7126a29b6d6d77bdfd0743)) - yuxuetr
- **(tools)** run_shell / write_file 描述如实说明现有的审批与路径限制 - ([e1f2279](https://github.com/yuxuetr/seekcli/commit/e1f2279b3888097d71b198525fdf0e0ac501d071)) - yuxuetr

### Documentation

- **(L6)** --output json 的形状订正为实际输出 - ([454b09a](https://github.com/yuxuetr/seekcli/commit/454b09ac4b20cba2ddb6e8b009303523ca498891)) - yuxuetr
- **(agents)** 工程品格补一条「判据也要被验」 - ([ee2c262](https://github.com/yuxuetr/seekcli/commit/ee2c262706608b0351f068b494a410bb694feada)) - yuxuetr
- **(agents)** AGENTS.md 压回 8 KB 注入上限之内 - ([4627306](https://github.com/yuxuetr/seekcli/commit/46273061aae2a3d16604b742e60f076904eab995)) - yuxuetr
- **(architecture)** 补齐阶段五十三，并订正一条当天就过期的结论 - ([e8411ea](https://github.com/yuxuetr/seekcli/commit/e8411eaad4e193852751f5f41bb665b9eff869b6)) - yuxuetr
- **(evaluation)** 五评——核查运行时目录，发现零真实使用 - ([c4b7449](https://github.com/yuxuetr/seekcli/commit/c4b7449b98aa3f83764f7e24ac2c3bcfd0097c35)) - yuxuetr
- **(evaluation)** 更正五评——使用不是没有过，是在特定一天停止 - ([a135da8](https://github.com/yuxuetr/seekcli/commit/a135da824ad1698b80d29d805d807f4118590797)) - yuxuetr
- **(evaluation)** 第四轮真实验收——四批「未验证」全部落地 - ([ec7d113](https://github.com/yuxuetr/seekcli/commit/ec7d11370ce86e2064efdcf2ed4ca087340e5d54)) - yuxuetr
- **(evaluation)** 六评——架构评审，并刷新过期的层完成度 - ([c1fbc8d](https://github.com/yuxuetr/seekcli/commit/c1fbc8df89a7c5107008041827d6f74a0210bf05)) - yuxuetr
- **(evaluation)** 七评——对照 Claude Code 扩展体系 - ([11fbf8d](https://github.com/yuxuetr/seekcli/commit/11fbf8d8ca8a9fa5ce868af293b558961433a0ea)) - yuxuetr
- **(evaluation)** 七评 §3 重估条件改为可执行脚本，补落地记录 - ([1530026](https://github.com/yuxuetr/seekcli/commit/15300261e573efa7459b3e4b8bc7bcbd0cb92c1d)) - yuxuetr
- **(readme)** 补齐漂掉的一半，并钉住不再漂 - ([b484b96](https://github.com/yuxuetr/seekcli/commit/b484b96fead422d59538d0db8b6db4a16d5bf041)) - yuxuetr
- **(release)** 阶段三十三收尾，并把两次 CI 失败的共同教训写进四评 - ([45ac6e7](https://github.com/yuxuetr/seekcli/commit/45ac6e7e6f99474acaa393161259e62c87c7eb31)) - yuxuetr
- **(render)** 阶段五十四 —— 补上阶段七留下的那半条 - ([a0fa2be](https://github.com/yuxuetr/seekcli/commit/a0fa2be734b50230226c11d7f0cf38e1f227001d)) - yuxuetr
- **(render)** 记下跨流顺序这个陷阱，以及为什么它需要一双眼睛 - ([caa1933](https://github.com/yuxuetr/seekcli/commit/caa19330a999e778819d9d19390cfb96496c3cea)) - yuxuetr
- **(shell)** 阶段五十五，并订正一条假的「已落地」 - ([6cfc335](https://github.com/yuxuetr/seekcli/commit/6cfc335d3ab43536a4d27d71e9f30419af30fbb7)) - yuxuetr
- **(site)** 用 mdBook 把文档发布到 GitHub Pages - ([26a2143](https://github.com/yuxuetr/seekcli/commit/26a214398f282c8fc06a71bc77b134a4404da223)) - yuxuetr
- **(skill)** doc_parser 改写为「flash 看懂 / MinerU 抄准」分工 - ([6d7d3cd](https://github.com/yuxuetr/seekcli/commit/6d7d3cde18cb54bbec995fc20e37abd601eff2ad)) - yuxuetr
- **(skill)** doc_parser 按实测改写分工——清晰截图上差的是成本，不是准确率 - ([5740cd2](https://github.com/yuxuetr/seekcli/commit/5740cd2dae3335c9b9ec758a2bdc927b7891747e)) - yuxuetr
- 阶段五十五收尾 —— 超时清理、审批提示的 Ctrl-C、一条既有竞态 - ([60ba323](https://github.com/yuxuetr/seekcli/commit/60ba323245ff807f1be9391d4ff3d965a69af59e)) - yuxuetr
- 阶段五十六 —— 记下一次真实使用暴露的三件事 - ([6ad2611](https://github.com/yuxuetr/seekcli/commit/6ad261194ee13c2b2080807ab0a9cdd8c22d4086)) - yuxuetr
- L6 §4.9 工具输出可见 —— 两条路，一条默认一条按需 - ([5fe07dd](https://github.com/yuxuetr/seekcli/commit/5fe07dd643321b7d1d4bd49a80ee32acc056668f)) - yuxuetr
- 56.2 收尾，并给 security-model 的「不是不可绕过」加一条限定 - ([1d72ed5](https://github.com/yuxuetr/seekcli/commit/1d72ed5ace75a8b62c8f8e9846bb5e95ed05f9e9)) - yuxuetr
- 49.2 —— 场景套件与能力套件是两回事，写清楚 - ([a6a6d5f](https://github.com/yuxuetr/seekcli/commit/a6a6d5f26a16c5aa3f77fface196c4bb8af454cd)) - yuxuetr
- 修复 16 处失效锚点与 TODOs 中未转义的 <server> - ([820e6cf](https://github.com/yuxuetr/seekcli/commit/820e6cfcadc429b3fa16fa9748149e996f6b814a)) - yuxuetr
- README / AGENTS.md / 配置模板同步阶段五十八 - ([db679a5](https://github.com/yuxuetr/seekcli/commit/db679a596814cb5b232aa5d639545e80c5aa0363)) - yuxuetr
- 记录验收命令的真实模型端到端运行 - ([42f68b7](https://github.com/yuxuetr/seekcli/commit/42f68b7be06ec376a495ee2ac9bd2991c0a147fd)) - yuxuetr

### Features

- **(bench)** 场景化评估套件，并修好一道本来就失效的闸门 - ([721d603](https://github.com/yuxuetr/seekcli/commit/721d603b4ea8e99bcd4a16cc829ee86219fc42ee)) - yuxuetr
- **(bench)** 编程场景的口径（49.2），四条任务全部来自真实失败 - ([07c9b96](https://github.com/yuxuetr/seekcli/commit/07c9b96312a38b8b64435b379dcf175b7bc7cddf)) - yuxuetr
- **(engine)** 整次运行的模型调用上限，并写入工程品格标准 - ([0b595ae](https://github.com/yuxuetr/seekcli/commit/0b595ae94e2185279454e9458002913d7ff300cd)) - yuxuetr
- **(engine)** 并行子代理 —— 三个 explore 从 33s 降到 16s - ([9abeca3](https://github.com/yuxuetr/seekcli/commit/9abeca36da6d94a463370ee359217ab10f91a0c6)) - yuxuetr
- **(engine)** 停止时由 harness 跑验收命令（[verify] command） - ([8bc4865](https://github.com/yuxuetr/seekcli/commit/8bc48657417054d037c13a870a36f20b416dac57)) - yuxuetr
- **(inspect)** 扩展装配做成一张表，而不是一套生命周期机制 - ([27fc825](https://github.com/yuxuetr/seekcli/commit/27fc825893e3a59dd0b79158c7b3eefb44e48bea)) - yuxuetr
- **(memory)** 压缩阈值随配置窗口派生，不再是全局常量 - ([c468d1d](https://github.com/yuxuetr/seekcli/commit/c468d1d4f58d56325df97cd6a18dcd9be5baa249)) - yuxuetr
- **(memory)** 跨会话分档记忆，偏好经闸门、领域可直写 - ([5bd6bb5](https://github.com/yuxuetr/seekcli/commit/5bd6bb5184632bbccb166ffecb2af6babbdf511b)) - yuxuetr
- **(planning)** 宏触发拿到自己的开关，与 thinking 解耦 - ([ea7c4e6](https://github.com/yuxuetr/seekcli/commit/ea7c4e6dcf8867a7a596a7e100c0587d12446304)) - yuxuetr
- **(prompt)** 告诉模型同一轮的委派会并发，并订正过期的委派理由 - ([fc07fe6](https://github.com/yuxuetr/seekcli/commit/fc07fe69ebcc5b067d0a09b291549f97bd1b684f)) - yuxuetr
- **(prompt)** 注入已安装 skill 的一行索引，让 load_skill 可被发现 - ([573cf56](https://github.com/yuxuetr/seekcli/commit/573cf567f4641bc797e26b596639f084d5b596fd)) - yuxuetr
- **(render)** 代码块顶格，并把 /copy 的序号打到屏幕上 - ([cbbf8d2](https://github.com/yuxuetr/seekcli/commit/cbbf8d253876c9c1535814e177b4173ce4977dba)) - yuxuetr
- **(repl)** Ctrl+V 贴图 - ([08c7276](https://github.com/yuxuetr/seekcli/commit/08c7276a67c58127b6c8853c4dbc3f8559ec0ab4)) - yuxuetr
- **(repl)** 会话续接补齐 —— /resume 无参数、--continue、--resume - ([375c2c8](https://github.com/yuxuetr/seekcli/commit/375c2c86331cb513ce58e57c827242f8546e5ec0)) - yuxuetr
- **(session)** 确立落盘顺序，并让崩溃留下的悬空调用可被识别 - ([a7c2a63](https://github.com/yuxuetr/seekcli/commit/a7c2a63e2309ddf6abf34e54d9313ec25677a968)) - yuxuetr
- **(session)** Run 身份统一——记账事件、子代理归属、trace join key - ([dc80b3e](https://github.com/yuxuetr/seekcli/commit/dc80b3ea62470b4a275eed9531b4730707b4c66d)) - yuxuetr
- **(session)** 逐轮 token 用量进日志，让一个死变体活过来 - ([ee9213b](https://github.com/yuxuetr/seekcli/commit/ee9213b7a8b130bc2af2363b7467ca655090512b)) - yuxuetr
- **(session)** /bad —— 阶段零的产出物此前无处可放 - ([1576b4c](https://github.com/yuxuetr/seekcli/commit/1576b4c7d3fd1c8b79e2e31b19db0e070a9f138c)) - yuxuetr
- **(session)** 图像投影时附上文件路径，让模型能把图交给 MinerU - ([a3353ba](https://github.com/yuxuetr/seekcli/commit/a3353baae37f4b9e3185182122058b212c62913c)) - yuxuetr
- **(shell,commands)** 工具输出对用户可见 —— 回显尾部，并新增 /output - ([f4ef5b5](https://github.com/yuxuetr/seekcli/commit/f4ef5b54d5703c1408e726604988ef255e7433cf)) - yuxuetr
- **(skill)** 新增 finance skill，以 Tushare 为数据源 - ([e6188ca](https://github.com/yuxuetr/seekcli/commit/e6188caa9c447640664da730a42dd7154cb2343c)) - yuxuetr
- **(skills)** allowed_tools 真正生效，裁剪 effective 工具面 - ([88a4686](https://github.com/yuxuetr/seekcli/commit/88a4686e2e3c2499a46f91bf9431a723b00957dc)) - yuxuetr
- **(subagents)** 无法解析的 prompt 要响亮地失败 - ([6d742c0](https://github.com/yuxuetr/seekcli/commit/6d742c0c9a9212ae27d2242785f6406b2c339ad4)) - yuxuetr
- **(tools)** web_search / web_fetch，带不可信内容围栏与来源记账 - ([09916fd](https://github.com/yuxuetr/seekcli/commit/09916fd7868cf75fd3146003b273c01d963aaa95)) - yuxuetr
- **(trace)** 子代理内部不再是盲区 - ([16a4574](https://github.com/yuxuetr/seekcli/commit/16a4574bf7dd86feb22d52ce1e396c04c59d04e9)) - yuxuetr
- **(trace)** /trace —— 记了要看得见 - ([edf70e4](https://github.com/yuxuetr/seekcli/commit/edf70e4c11cd9f58a2617e29d6b5bc5ac3c7e8e2)) - yuxuetr
- **(trace)** 追踪默认打开，并限制保留数 - ([d7258f7](https://github.com/yuxuetr/seekcli/commit/d7258f7ca41f1f8234b9aed9d3916eebbb13e053)) - yuxuetr

### Miscellaneous Chores

- **(deps)** 升级 rustls 0.23.45 → 0.23.46 - ([8345508](https://github.com/yuxuetr/seekcli/commit/834550835a3cdcc675708eb70c7b876e52a2edd1)) - yuxuetr
- skill 不随仓库发布，移除 examples/skills - ([c42d5c9](https://github.com/yuxuetr/seekcli/commit/c42d5c9ebab930c5cc2343e1112247230349fed8)) - yuxuetr

### Other

- **(deps)** rustls 0.23.40 -> 0.23.45，修掉 GHSA-2mjx-qc3c-rqvc - ([8c80597](https://github.com/yuxuetr/seekcli/commit/8c80597e658beb16886eafdd6d0fd9d22b37635a)) - yuxuetr

### Performance

- **(session)** 事件日志真正增量 append，不再整文件重写 - ([a0b2c3f](https://github.com/yuxuetr/seekcli/commit/a0b2c3fc44abc990d11e682aa84aa22adf88d718)) - yuxuetr

### Refactoring

- **(engine)** 抽出工具面装配，主循环 473 -> 431 行 - ([70c04de](https://github.com/yuxuetr/seekcli/commit/70c04de9ee2acf1c4ca748e1cca8f223c606044a)) - yuxuetr
- 抑制注解改用 #[expect]，并清掉两条过期的 - ([e5a81ef](https://github.com/yuxuetr/seekcli/commit/e5a81ef161aeee7b462dd9893f161d0cfb284bc2)) - yuxuetr

### Tests

- **(bench)** programming.json 实跑 4/4 —— 以及这个满分为什么要打折看 - ([1b9b730](https://github.com/yuxuetr/seekcli/commit/1b9b730af217f7d69c5ad650b9da35bb365d5a76)) - yuxuetr
- **(bench)** denied_shell_write 的断言接受中文解释 - ([5f7c2a3](https://github.com/yuxuetr/seekcli/commit/5f7c2a31beb8c302fdf9ed360ee71d1ab1e9aa18)) - yuxuetr
- **(loop)** scratch 目录建不起来时响亮失败，不再静默跑进仓库 - ([60ed5d0](https://github.com/yuxuetr/seekcli/commit/60ed5d0a1171ef8f1085befc3ccbe4b7c799aad4)) - yuxuetr

---
## [0.2.0](https://github.com/yuxuetr/seekcli/compare/v0.1.0..v0.2.0) - 2026-09-13

### Bug Fixes

- **(agent)** defer load_skill's system message push until after tool responses - ([21c6f09](https://github.com/yuxuetr/seekcli/commit/21c6f09e5df329aff15a1fa6baec82c72a6f2488)) - yuxuetr
- **(api)** strip history reasoning from OpenAI requests too - ([da5dce2](https://github.com/yuxuetr/seekcli/commit/da5dce2295775093eb4500d47126ca766b2d2295)) - yuxuetr
- **(changelog)** cliff.toml 的仓库 URL 指向本仓库，并重新生成 CHANGELOG - ([7e6c6cd](https://github.com/yuxuetr/seekcli/commit/7e6c6cdf9b40590ea5b5289533baea76b44dedd0)) - yuxuetr
- **(clippy)** resolve unused import, variable, and dead code warnings - ([53f3abc](https://github.com/yuxuetr/seekcli/commit/53f3abc2681fab0c07d9a7be9bd8b464bee1ac05)) - yuxuetr
- **(commands)** `std::io::Write` 收进 macOS 分支，修掉 Linux 上的零警告门禁 - ([1ac1eea](https://github.com/yuxuetr/seekcli/commit/1ac1eeacbb16b0cb3157a490879989d63f579a8f)) - yuxuetr
- **(config)** 配置改为用户级 + 项目级覆盖，不再读写当前目录 - ([fd91a54](https://github.com/yuxuetr/seekcli/commit/fd91a54b052dbd22aa26a6b4855e207fb03c66ff)) - yuxuetr
- **(config)** 纠正已变假的前提 —— 模型名、vision skill、MCP 归因 - ([7e1ecb5](https://github.com/yuxuetr/seekcli/commit/7e1ecb5684f07dc0519e4dad3458dd1be0bc93be)) - yuxuetr
- **(debug)** reduce excessive redundant debug information in third-party libraries - ([c9f1ddf](https://github.com/yuxuetr/seekcli/commit/c9f1ddfae98cef967f4418e38c7927f5a8530a35)) - yuxuetr
- **(policy)** 拒绝消息改为可行动，不再谎称工具不可用 - ([1d16c2f](https://github.com/yuxuetr/seekcli/commit/1d16c2fd6961866ca651e1cf0a57f3a3e008392c)) - yuxuetr
- **(skills)** drop default skills + add create_skill name collision check - ([6e6e8a5](https://github.com/yuxuetr/seekcli/commit/6e6e8a551abdc95242c1697a0c6b2ca8ea9a2452)) - yuxuetr
- **(skills)** use step-1o-vision-32k (step-1.5v-mini was retired) - ([e640406](https://github.com/yuxuetr/seekcli/commit/e640406fcdec4fbe06dab2843f5047b35e9a18ec)) - yuxuetr
- **(skills)** always push SKILL.md prompt on activation + dedup on switch - ([fab9b6b](https://github.com/yuxuetr/seekcli/commit/fab9b6ba5b85d32cde4546fbb36d22b8054350ad)) - yuxuetr
- **(skills)** doc_parser MinerU upload + tighten fallback policy - ([bd58cec](https://github.com/yuxuetr/seekcli/commit/bd58cec4a765d2862338b5481eaaba69c7e8a3db)) - yuxuetr
- **(skills)** align mineru_parse.sh with MinerU V4 docs (state machine + limits) - ([5464209](https://github.com/yuxuetr/seekcli/commit/5464209f8d2d1e820c7d23f5b596f21a52ff2db4)) - yuxuetr
- robust image processing in paste_image to avoid bus error - ([45f4ee0](https://github.com/yuxuetr/seekcli/commit/45f4ee0196d77649ac3a992936324d328c5237c5)) - yuxuetr
- use spawn_blocking for clipboard and fix arboard error matching - ([ea9f1d7](https://github.com/yuxuetr/seekcli/commit/ea9f1d762e4924b991d95094c1501175869adb10)) - yuxuetr
- resolve cargo-deny advisories by updating dependencies and ignoring unmaintained transitive crates - ([8ff0b56](https://github.com/yuxuetr/seekcli/commit/8ff0b564434b61ec50c0667b7a431b5cac0c06d4)) - yuxuetr
- resolve compilation error and apply formatting for JINA_API_KEY support - ([899c735](https://github.com/yuxuetr/seekcli/commit/899c7353711dc7fd3ed513bcf6da5fded5b727fc)) - yuxuetr
- correct GLM web search API endpoint and payload - ([9970faf](https://github.com/yuxuetr/seekcli/commit/9970faf1a9b2ec6a8c1a5058f1413957dac5036a)) - yuxuetr
- add dedicated glm_sensor with search_pro support and fix 401 error - ([fb1ed00](https://github.com/yuxuetr/seekcli/commit/fb1ed007bc59d7ec4096756938431f20cf87fffc)) - yuxuetr
- update GLM search response parsing to use search_result field - ([3c53271](https://github.com/yuxuetr/seekcli/commit/3c532717cd9f0be945b6578db0589852e11130c8)) - yuxuetr

### Documentation

- **(arch)** declare L3 security boundary; refresh layer evaluation - ([9f0a29e](https://github.com/yuxuetr/seekcli/commit/9f0a29ebc80c4ec2e9c3d880328dd1c70ac6466b)) - yuxuetr
- **(architecture)** update module layout for main.rs split - ([2ebf300](https://github.com/yuxuetr/seekcli/commit/2ebf3003ba87aa728969fe07ca3a3fc81a571dad)) - yuxuetr
- **(architecture)** 把 dsh 判据从「太复杂」升级为可判定的技术判据 - ([11a8129](https://github.com/yuxuetr/seekcli/commit/11a81299dbebb0e1690c89e3d54ccef5c871aa4c)) - yuxuetr
- **(architecture)** 补 L1-6 教学式错误的目标设计 - ([7d66791](https://github.com/yuxuetr/seekcli/commit/7d66791ae64e5642a9b67e1ea63df22ac1713d9c)) - yuxuetr
- **(architecture)** 补 L7-6 自描述的目标设计，并登记 L7-7 / L7-8 - ([3160b9f](https://github.com/yuxuetr/seekcli/commit/3160b9f0a9f6ff44f731079fa7f17a137b16cf90)) - yuxuetr
- **(architecture)** 补 L5-6 提案闸门通用化的目标设计 - ([2d8ea08](https://github.com/yuxuetr/seekcli/commit/2d8ea08bdefea16b9230a2614017924837d7f268)) - yuxuetr
- **(architecture)** 补 L7-8 trajectory 导出的目标设计 - ([2cc2bd7](https://github.com/yuxuetr/seekcli/commit/2cc2bd75cea6c35e53cb2534b3ac5dfbec683a83)) - yuxuetr
- **(architecture)** 划清「能力」与「用户输入」的界，并补 L4-8 设计 - ([bfb324c](https://github.com/yuxuetr/seekcli/commit/bfb324cfc477e6beea0422d89d9d53c151166118)) - yuxuetr
- **(architecture)** 记录 MCP 图像透传的实测结论与未决的设计选择 - ([5d4f38b](https://github.com/yuxuetr/seekcli/commit/5d4f38b7e6ca6c3d30981672d75332c470f55f7f)) - yuxuetr
- **(architecture)** 补 L7-7 eval 回灌的目标设计，并收窄适用面 - ([4e3199b](https://github.com/yuxuetr/seekcli/commit/4e3199b5a3caf0789ec359b47176644cdf2474f9)) - yuxuetr
- **(changelog)** regenerate for stages 13-17 - ([f26f34c](https://github.com/yuxuetr/seekcli/commit/f26f34c139778c7300e6df9e9d00fb4e9ff07831)) - yuxuetr
- **(changelog)** regenerate to include edit_file docs commit - ([ba49281](https://github.com/yuxuetr/seekcli/commit/ba4928106dfce34d06a24134c10c63ff10c7a995)) - yuxuetr
- **(evaluation)** 复评 —— 对照 deepseek-harness 从 40% 到 80% - ([f0d3784](https://github.com/yuxuetr/seekcli/commit/f0d37845abb1b25163b9bd8d668a2008ded6e937)) - yuxuetr
- **(evaluation)** 三评引入 D 口径与第二轮路线图 - ([856a37c](https://github.com/yuxuetr/seekcli/commit/856a37cf9edf25cadf9b1708ca9fd91f2943f660)) - yuxuetr
- **(evaluation)** 记录基底变更 —— deepseek-flash 具备视觉 - ([3ce2549](https://github.com/yuxuetr/seekcli/commit/3ce254983701d4d25fc9665fd20b79ceb865d218)) - yuxuetr
- **(evaluation)** 四评 —— 两轮推进后的 D 口径复盘 - ([04f72d0](https://github.com/yuxuetr/seekcli/commit/04f72d024f78183d9d5e7b3a2d8766451e7443d2)) - yuxuetr
- **(readme)** document stages 13-17 harness features - ([5304819](https://github.com/yuxuetr/seekcli/commit/5304819b8295e99ae1db63ce2a1d452d0a979907)) - yuxuetr
- update README with multimodal sensors, @ syntax, and dual execution modes - ([46d8488](https://github.com/yuxuetr/seekcli/commit/46d84884e080b9c32d59e1414a9852e4e971a692)) - yuxuetr
- update TODOs.md to mark completed milestones - ([ac6202d](https://github.com/yuxuetr/seekcli/commit/ac6202dce8f96704c7dd5287b7adab68533c5d7e)) - yuxuetr
- mark Phase 5 tasks as completed - ([4796547](https://github.com/yuxuetr/seekcli/commit/4796547d9357eb34050638d0a55fcb20410daae6)) - yuxuetr
- sync TODOs/README/CHANGELOG with phase 7 completion - ([67b3226](https://github.com/yuxuetr/seekcli/commit/67b32260c2c03bd71dc3b01306d684356d038603)) - yuxuetr
- mark stage 17 (L7 observability) and figure-3 checklist complete - ([ea3c9e7](https://github.com/yuxuetr/seekcli/commit/ea3c9e7b5a64119401a454437562aae3c8db3897)) - yuxuetr
- document dual OpenAI/Anthropic provider support - ([4bdb848](https://github.com/yuxuetr/seekcli/commit/4bdb8480839e5da2afe707bba58bc25efdff60f6)) - yuxuetr
- README + CHANGELOG for dual provider & reasoning handling - ([4ce372b](https://github.com/yuxuetr/seekcli/commit/4ce372b7fb1b7110b527b82a91eae90de7f5c338)) - yuxuetr
- document edit_file tool - ([00c99ca](https://github.com/yuxuetr/seekcli/commit/00c99cab1e718eef15baecf2695b9aeac6021654)) - yuxuetr
- 重构文档结构并按 deepseek-harness 对照重新规划路线图 - ([b76af26](https://github.com/yuxuetr/seekcli/commit/b76af26f301b7a6f380bdfe4fc72ddd92f6784f2)) - yuxuetr
- 补 MIT LICENSE 文件 - ([01221e9](https://github.com/yuxuetr/seekcli/commit/01221e97eb97a92c5d77962aafa22df366b4574d)) - yuxuetr
- 补齐评估到路线图的溯源，并加机械校验 - ([42de7e8](https://github.com/yuxuetr/seekcli/commit/42de7e8f337f1b1c35d10e534742000f9f8b9b36)) - yuxuetr
- 修正层文档与路线图的状态漂移，并让校验器能抓到它 - ([e2cdc0a](https://github.com/yuxuetr/seekcli/commit/e2cdc0a948f5ef12d621773b6189949e70f767ed)) - yuxuetr
- 记录不发布 crates.io 的决定并修复安装指令 - ([adc7d52](https://github.com/yuxuetr/seekcli/commit/adc7d52e142c5af4d341777baf26cc6889807353)) - yuxuetr

### Features

- **(agent)** wire harness core, fix tool streaming - ([61c2ec4](https://github.com/yuxuetr/seekcli/commit/61c2ec4b80166fec01b6976efb427f288afc23a3)) - yuxuetr
- **(agent)** context-aware Error Recovery hints + dispatcher BAD ARGS fix - ([fbebc58](https://github.com/yuxuetr/seekcli/commit/fbebc583632c51a7d5fbf07217c265702b8db0d4)) - yuxuetr
- **(agent)** System Reminders to break tool-call doom loops - ([07f2afb](https://github.com/yuxuetr/seekcli/commit/07f2afb6e62e7602db12e6668dd8cfe17a009b76)) - yuxuetr
- **(agent)** Fork-Join — read-concurrent, write-serial tool execution - ([651696c](https://github.com/yuxuetr/seekcli/commit/651696cb0fb889922861e4ec846358f10dbb6a04)) - yuxuetr
- **(agent)** dynamic Two-Stage ReAct planning phase - ([be5b0b0](https://github.com/yuxuetr/seekcli/commit/be5b0b04cbbc726404fee436cfc2b0e64247b0d9)) - yuxuetr
- **(agent)** Plan Mode — externalize long-task state to PLAN.md/TODO.md - ([0d0eaf1](https://github.com/yuxuetr/seekcli/commit/0d0eaf1d848b89adde52d72798795551c431a563)) - yuxuetr
- **(api)** Anthropic-compatible provider (DeepSeek /anthropic) - ([7444fd4](https://github.com/yuxuetr/seekcli/commit/7444fd4e3b80791e12a5580181a50c4b12583bd1)) - yuxuetr
- **(api)** L0 韧性装饰器与 provider 配置化 - ([d03e30d](https://github.com/yuxuetr/seekcli/commit/d03e30d4a4fb62343cc2b9c72d58ff54d83acd22)) - yuxuetr
- **(api,session)** 多段内容 —— Message 带图像，事件日志存 blob 引用 - ([5f9df1b](https://github.com/yuxuetr/seekcli/commit/5f9df1bcd88750609e5499ceafe16bfd7304948f)) - yuxuetr
- **(bench)** trajectory 导出 —— 让外部 RL / 进化流程能消费 bench 运行 - ([ad91414](https://github.com/yuxuetr/seekcli/commit/ad91414e8e0fb2a773e397c90f92d7d8a6ba46ad)) - yuxuetr
- **(cli)** headless 通用化 —— -p / stdin / --output json / 退出码 - ([215bbed](https://github.com/yuxuetr/seekcli/commit/215bbed0d17462fd438abfceff0e6ef144fd49bc)) - yuxuetr
- **(cost)** persist per-session cost to session JSON; scope to session - ([2741d2a](https://github.com/yuxuetr/seekcli/commit/2741d2ae25a3da3c140a3aea8c7aacdfe966772c)) - yuxuetr
- **(eval)** eval 套件扩到 26 个任务，含反向安全断言 - ([2a1b259](https://github.com/yuxuetr/seekcli/commit/2a1b25981fca4896dffebaf405b7b4540974760b)) - yuxuetr
- **(mcp)** MCP 客户端 —— 第三方终于能扩展 SeekCLI - ([0ddbaf0](https://github.com/yuxuetr/seekcli/commit/0ddbaf02a91c0c956b8b9f61dd8533d16640df65)) - yuxuetr
- **(mcp)** 启动失败记账，并在模型碰到缺失工具时讲清原因 - ([7ca1540](https://github.com/yuxuetr/seekcli/commit/7ca1540dca38f5b78e81f4344ffd204cafc132c4)) - yuxuetr
- **(mcp)** 图像透传 —— 工具返回的截图真的到达模型 - ([062f5a3](https://github.com/yuxuetr/seekcli/commit/062f5a392c40a7638d5e44dffc85d83d307a5048)) - yuxuetr
- **(memory)** context compression + prompt cache usage logging (Phase 10) - ([af5b461](https://github.com/yuxuetr/seekcli/commit/af5b461a9cf797e3aad3cfd850d8422aef08644f)) - yuxuetr
- **(memory)** staged-degradation context compression - ([440d659](https://github.com/yuxuetr/seekcli/commit/440d65973d25190479e7738b989b17af38b4b0e9)) - yuxuetr
- **(observability)** CostTracker — session token/CNY accounting (17.1) - ([d2380a0](https://github.com/yuxuetr/seekcli/commit/d2380a06924174e1d6bcbd820ee111163e8be94c)) - yuxuetr
- **(observability)** decision-path tracing to .claw/traces (17.2) - ([c32c6ea](https://github.com/yuxuetr/seekcli/commit/c32c6eaa1a0a3b19c9f6f190b8125f587825fe64)) - yuxuetr
- **(observability)** Benchmark Runner — Fail-to-Pass agent evaluation (17.3) - ([acf2dae](https://github.com/yuxuetr/seekcli/commit/acf2daeb7fefc0037c8fc7bcee82bcb2d22e1841)) - yuxuetr
- **(paste)** /paste 剪贴板图像输入，并给图像一个 token 下限 - ([6a84bcf](https://github.com/yuxuetr/seekcli/commit/6a84bcf6be87b91ed5a800e281971e3c058d5675)) - yuxuetr
- **(prompt)** dynamic Prompt Composer reads workspace AGENTS.md/CLAUDE.md - ([4cec5e2](https://github.com/yuxuetr/seekcli/commit/4cec5e293c26afd2ae889f209c9c4aa975180025)) - yuxuetr
- **(proposals)** 提案闸门通用化 —— 模型能提 MCP server 与定时任务 - ([c57d397](https://github.com/yuxuetr/seekcli/commit/c57d397cbf1a07b47bc423d61d69b4db18c0ae1d)) - yuxuetr
- **(propose)** eval 回灌闸门 —— 接受一个 skill 前先证明它没让 agent 变差 - ([c8581a2](https://github.com/yuxuetr/seekcli/commit/c8581a2af1a03fc8147305ba23c804b5a19c4a2b)) - yuxuetr
- **(repl)** /skill <name> <prompt> activates AND sends prompt in one line - ([3c3bf71](https://github.com/yuxuetr/seekcli/commit/3c3bf710ee9cef28c2e96dc54d13b1190c0374cf)) - yuxuetr
- **(safety)** add danger approval + path whitelist (Phase 8) - ([9a3f8b5](https://github.com/yuxuetr/seekcli/commit/9a3f8b5a719f96d863d706761dcf216edc6f3b06)) - yuxuetr
- **(security)** three-state allow/ask/deny command policy - ([1b18b64](https://github.com/yuxuetr/seekcli/commit/1b18b64465f95db54543d98e0cfaaa368b2f61eb)) - yuxuetr
- **(security)** 统一策略门、子命令判定、越界写检测与审计日志 - ([0e72f46](https://github.com/yuxuetr/seekcli/commit/0e72f4676a72c90b747cc3779181e2391768deea)) - yuxuetr
- **(session)** 压缩事件化、offload 归属会话、阈值改用 token - ([f49e0dd](https://github.com/yuxuetr/seekcli/commit/f49e0dd69b6c3a2982b9e3c862a5f72fbcde9328)) - yuxuetr
- **(skills)** proposal-review flow + load_skill tool (Phase 9.2 + 9.3) - ([1c4910b](https://github.com/yuxuetr/seekcli/commit/1c4910bc19ce5ca9e45f086a99c0633677c7ef34)) - yuxuetr
- **(skills)** SKILL.md directory format parser + dual-format loader (Phase 12.1/12.2) - ([3169e9a](https://github.com/yuxuetr/seekcli/commit/3169e9a2988da3eb0b569738c88eadd7357bfd50)) - yuxuetr
- **(skills)** /skill migrate + create_skill writes SKILL.md (Phase 12.3/12.4) - ([aa4d046](https://github.com/yuxuetr/seekcli/commit/aa4d046747a8be687916c86b05fa3e87f5e06a73)) - yuxuetr
- **(skills)** scripts/references asset enrichment + docs (Phase 12.5/12.6) - ([7a88820](https://github.com/yuxuetr/seekcli/commit/7a88820d8b3fba0584f853eb555028c5551c5c8b)) - yuxuetr
- **(skills)** add vision + doc_parser example skills - ([9c21248](https://github.com/yuxuetr/seekcli/commit/9c21248f05df464bbdc918d778e4ff1748d27ff2)) - yuxuetr
- **(skills)** doc_parser handles OCR (clipboard + image path) via MinerU - ([7283caa](https://github.com/yuxuetr/seekcli/commit/7283caa08e0ca885c38b457b56385ef785d19cf6)) - yuxuetr
- **(subagents)** typed templates with allowed_tools whitelist (Phase 9.1) - ([4cccfff](https://github.com/yuxuetr/seekcli/commit/4cccfffc420cfefe120f0cf64ebc05fbf0ae6bcd)) - yuxuetr
- **(tasks)** L8 循环层 headless 定时任务 + Two-Stage 假工具调用修复 - ([52dd8da](https://github.com/yuxuetr/seekcli/commit/52dd8da1931a8d1b6fa252827fd4c11c5d934556)) - yuxuetr
- **(tasks)** L8 任务改为声明式 TASK.md - ([5ee68c5](https://github.com/yuxuetr/seekcli/commit/5ee68c53b271c896f858a2062add0fb2d7bbea79)) - yuxuetr
- **(tools)** offload oversized tool output to temp file with preview - ([5aa4abc](https://github.com/yuxuetr/seekcli/commit/5aa4abcea6150cd045466710471375350f91ac80)) - yuxuetr
- **(tools)** add edit_file with L1-L4 fuzzy-match chain - ([209cb76](https://github.com/yuxuetr/seekcli/commit/209cb7684764b7c37f60bcad5079a823db5e6a51)) - yuxuetr
- **(tools)** 新增原生 glob / grep 检索工具 - ([86e3ec0](https://github.com/yuxuetr/seekcli/commit/86e3ec050bfa121ce8d052a2299b6fc477dd0976)) - yuxuetr
- **(tools)** 结构化工具结果、统一执行管线与 ask_user_question - ([9f180e2](https://github.com/yuxuetr/seekcli/commit/9f180e22d15882765471aa7569e4c84f7ecc07a8)) - yuxuetr
- **(tools)** 后台任务与 job_list / job_output / job_kill - ([c0dbdb9](https://github.com/yuxuetr/seekcli/commit/c0dbdb94348928e89a85c0032edda37774daf1a8)) - yuxuetr
- **(tools)** 未知工具名给出最接近的候选与调用签名 - ([f1827de](https://github.com/yuxuetr/seekcli/commit/f1827de611daf60dba4d51ef006ab076baa93687)) - yuxuetr
- **(tools)** harness_inspect —— 让模型能查询自己的运行时 - ([c2f5c91](https://github.com/yuxuetr/seekcli/commit/c2f5c9148a84839a27f203760c012649e0308f18)) - yuxuetr
- **(tools)** read_image 与带图 eval，删除 vision skill；L4-8 结清 - ([bedf435](https://github.com/yuxuetr/seekcli/commit/bedf435f25b7bd47bf3619e6071477c37b97abb9)) - yuxuetr
- **(ui)** Ctrl-C interrupt + indicatif spinner for long shell (Phase 11 C1) - ([4c19a5e](https://github.com/yuxuetr/seekcli/commit/4c19a5efe4e7f9d992f9ba941e575641cfbc4c0c)) - yuxuetr
- **(ui)** rustyline tab completion + docs close out Phase 11 (C2) - ([3c3c1fe](https://github.com/yuxuetr/seekcli/commit/3c3c1fe8dc2c0aeef4f9c04e334d8cf2a8d74f27)) - yuxuetr
- implement multimodal API and Sensor-Brain vision pipeline - ([d6fe6c7](https://github.com/yuxuetr/seekcli/commit/d6fe6c7ae57775c47c99e40c80282a458afbdf61)) - yuxuetr
- integrate MinerU V4 API for PDF parsing and add configuration management - ([462e74a](https://github.com/yuxuetr/seekcli/commit/462e74af95a1f20287a8f3ed74af04b23283afa7)) - yuxuetr
- unify sensor data integration with @paste syntax and refactor paste_image - ([146613e](https://github.com/yuxuetr/seekcli/commit/146613eb3c400638eaa17bc5acec35ad8a07947b)) - yuxuetr
- add @image and @img as keywords for clipboard image analysis - ([f65a93d](https://github.com/yuxuetr/seekcli/commit/f65a93d15d065b7de30318d9feb3008f7d5e5928)) - yuxuetr
- rename /paste to /image and add /file subcommand for standalone analysis - ([13a8f5a](https://github.com/yuxuetr/seekcli/commit/13a8f5a21ea17f7aa54b9b98bf1efd472e9d4644)) - yuxuetr
- make /image and /file standalone tools that do not affect conversation history - ([b01c2b7](https://github.com/yuxuetr/seekcli/commit/b01c2b7f3d2ac73a901142e7806f51aa3dc72f67)) - yuxuetr
- support quoted paths and spaces in commands and @ references - ([4ff5b39](https://github.com/yuxuetr/seekcli/commit/4ff5b395584b1ef51acad6caa59bc8396a26859f)) - yuxuetr
- implement Web Sense with @url and /web command using Jina Reader - ([9b68923](https://github.com/yuxuetr/seekcli/commit/9b68923bc5734cda65da73ffbe0bc303b3c2472e)) - yuxuetr
- integrate GLM and Tavily search sensors with standalone and inline modes - ([dfd511b](https://github.com/yuxuetr/seekcli/commit/dfd511bd2e7ddd953032c3b228fa37efa6dd334a)) - yuxuetr
- expose subcommands to top-level CLI for direct shell usage - ([eaab7a3](https://github.com/yuxuetr/seekcli/commit/eaab7a38475ae1d3babc3155e9282fcbd179d4c5)) - yuxuetr
- implement tool dispatcher and core system tools (fs, shell) - ([3ac3882](https://github.com/yuxuetr/seekcli/commit/3ac3882dd7228bf5cfdb41069d3f797b46a84ef1)) - yuxuetr
- implement ReAct loop in core chat engine - ([d439aa5](https://github.com/yuxuetr/seekcli/commit/d439aa5167c8025ce732c99d6516511498f60483)) - yuxuetr
- implement Sub-Agent delegation mechanism - ([b94c933](https://github.com/yuxuetr/seekcli/commit/b94c93379306eb4d56aca223a0a7fc2330f6c990)) - yuxuetr
- implement Dynamic Skill Generation mechanism - ([ffdb784](https://github.com/yuxuetr/seekcli/commit/ffdb78456e3c3e7bf5f31089b7c3fa15f2b3bb82)) - yuxuetr

### Miscellaneous Chores

- **(deps)** 修复失效的 cargo-deny 钩子并消除安全公告 - ([acc0366](https://github.com/yuxuetr/seekcli/commit/acc036606e6dfd54cc0b3d1196709ccf37cf0096)) - yuxuetr
- **(release)** 多平台产物、crates.io 元数据、CONTRIBUTING 与 issue 模板 - ([a078257](https://github.com/yuxuetr/seekcli/commit/a07825792c3a86d4f50c5bc1fc271e92a73936d3)) - yuxuetr
- **(release)** 版本号 0.1.0 → 0.2.0 - ([e2dcd0a](https://github.com/yuxuetr/seekcli/commit/e2dcd0a6ee2cfaa1ea3329a17e76f549189b19cd)) - yuxuetr
- gitignore .claude/ workspace state - ([183d377](https://github.com/yuxuetr/seekcli/commit/183d37764589448f62174dca4547fb641a09b250)) - yuxuetr

### Other

- **(ci)** update version for checkout and git-cliff - ([c3f1771](https://github.com/yuxuetr/seekcli/commit/c3f1771ee3f613df0217d61767108295e41b393d)) - yuxuetr
- **(ci)** update version for checkout and git-cliff again - ([bc1acfd](https://github.com/yuxuetr/seekcli/commit/bc1acfd287dd1dd46c7367fdcffa475e2a06d18c)) - yuxuetr
- **(deps)** reqwest 改用 rustls，修好 aarch64-linux 发布并去掉 libssl 运行时依赖 - ([0d0b985](https://github.com/yuxuetr/seekcli/commit/0d0b98572715381077e5cd211dd16c04dea947ee)) - yuxuetr
- update github actions build workflow rust setup configuration - ([41f35a2](https://github.com/yuxuetr/seekcli/commit/41f35a28931e8139c7d8002d015190856e2ce3c4)) - yuxuetr
- 接上覆盖率门禁并补仓库自身的 AGENTS.md - ([83b8862](https://github.com/yuxuetr/seekcli/commit/83b88623d8e8f9ef6f3c3994ce4223aee571fca0)) - yuxuetr

### Refactoring

- **(api)** drop multi-provider, multimodal, search; trim deps - ([fd24874](https://github.com/yuxuetr/seekcli/commit/fd2487461343e9d4d14da51ea66032e13d55cb37)) - yuxuetr
- **(api)** introduce LlmProvider trait; OpenAI client behind it - ([20f4302](https://github.com/yuxuetr/seekcli/commit/20f4302cffbb6a65823ce044a075c1a17faacc7c)) - yuxuetr
- **(engine)** 主循环 524 → 339 行，并让取消真正传到子进程 - ([bd6da8a](https://github.com/yuxuetr/seekcli/commit/bd6da8a32e07ff08bcb297da815d014244ded2fe)) - yuxuetr
- **(main)** strip peripheral sensors, slash commands, rendering - ([35e524c](https://github.com/yuxuetr/seekcli/commit/35e524cb2516b4068f70e9e2e16bc97e2148a250)) - yuxuetr
- **(main)** extract rustyline completer to completer.rs - ([9c08ecf](https://github.com/yuxuetr/seekcli/commit/9c08ecfc7d8bfab2dd185b197cbc0286078b98f8)) - yuxuetr
- **(main)** extract run_benchmark to benchmark.rs - ([346bf20](https://github.com/yuxuetr/seekcli/commit/346bf20a5627ed3a98b2648ebe794bd078841df5)) - yuxuetr
- **(main)** extract ReAct engine to engine.rs - ([6a473ca](https://github.com/yuxuetr/seekcli/commit/6a473ca74d5d155f73e3df5f0d88bfcd8c0c38db)) - yuxuetr
- **(main)** extract REPL command handling to commands.rs - ([71b7f44](https://github.com/yuxuetr/seekcli/commit/71b7f44e01e247283244f774a854e35fc22c7bf1)) - yuxuetr
- **(observability)** store traces under ~/.seekcli/traces - ([4bab2cb](https://github.com/yuxuetr/seekcli/commit/4bab2cb0d8656fb86274e98a974e3ba2a55e4ff5)) - yuxuetr
- **(session)** 会话改为 append-only 事件日志 - ([5bc69b7](https://github.com/yuxuetr/seekcli/commit/5bc69b78c685888adc4ea4ca5f052e22f3a45a0c)) - yuxuetr
- remove arboard and use system commands for clipboard, revert to multi-threaded tokio - ([1d77a42](https://github.com/yuxuetr/seekcli/commit/1d77a42e07e4472bf5d4764534f0403e6bf1f28e)) - yuxuetr

### Tests

- **(bench)** 把模型回答暴露给 eval，并补三条拒绝后行为断言 - ([01f924e](https://github.com/yuxuetr/seekcli/commit/01f924e80f0c9cdd92d0e2da971aec05b0172477)) - yuxuetr
- **(engine)** LLM 录制/回放，agent 主循环首次可测 - ([694802b](https://github.com/yuxuetr/seekcli/commit/694802bda595cdc325307a18d6c01634f9b2d5de)) - yuxuetr
- fix test_load_all_skills by asserting only default skills - ([2b8443e](https://github.com/yuxuetr/seekcli/commit/2b8443e4410832411c87a9a0c01d82bd66fbf160)) - yuxuetr
- 用真实 LLM 跑完验证，结清 L1-6 与 L7-6 - ([7b4d19b](https://github.com/yuxuetr/seekcli/commit/7b4d19b17041a193b2bc0967057c621927e173a4)) - yuxuetr

---
## [0.1.0] - 2026-04-29

### Documentation

- fix README.md and add CHANGELOG.md - ([d354109](https://github.com/yuxuetr/seekcli/commit/d3541095ee693fe1d8ccb1e4b889f055a381ad2a)) - yuxuetr
- update README and CHANGELOG, add development roadmap v0.1.0 - ([ac5bf82](https://github.com/yuxuetr/seekcli/commit/ac5bf8212c7052df8a6042a87026257683dfc50f)) - yuxuetr

### Features

- support deepseek chat and r1 model for cli - ([200af21](https://github.com/yuxuetr/seekcli/commit/200af215b4f6973871bc105f16210946e4316392)) - yuxuetr
- using dashscope api and support stream mode - ([c44c732](https://github.com/yuxuetr/seekcli/commit/c44c732ff73b440c7b24188c16dde06caf310ba3)) - yuxuetr
- add markdown rendering support via termimad and improve output display - ([8f7568f](https://github.com/yuxuetr/seekcli/commit/8f7568f883f200303cc6a22d8a3a1de089bdd9d3)) - yuxuetr
- implement syntax highlighting, code block extraction, and /copy command - ([6fd0d0a](https://github.com/yuxuetr/seekcli/commit/6fd0d0a086f469d4094be3935ecd73943dd16023)) - yuxuetr

<!-- generated by git-cliff -->
