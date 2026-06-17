# Changelog

All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

---
## [unreleased]

### Bug Fixes

- **(api)** strip history reasoning from OpenAI requests too - ([da5dce2](https://github.com/yuxuetr/rust-template/commit/da5dce2295775093eb4500d47126ca766b2d2295)) - yuxuetr
- **(clippy)** resolve unused import, variable, and dead code warnings - ([53f3abc](https://github.com/yuxuetr/rust-template/commit/53f3abc2681fab0c07d9a7be9bd8b464bee1ac05)) - yuxuetr
- **(debug)** reduce excessive redundant debug information in third-party libraries - ([c9f1ddf](https://github.com/yuxuetr/rust-template/commit/c9f1ddfae98cef967f4418e38c7927f5a8530a35)) - yuxuetr
- robust image processing in paste_image to avoid bus error - ([45f4ee0](https://github.com/yuxuetr/rust-template/commit/45f4ee0196d77649ac3a992936324d328c5237c5)) - yuxuetr
- use spawn_blocking for clipboard and fix arboard error matching - ([ea9f1d7](https://github.com/yuxuetr/rust-template/commit/ea9f1d762e4924b991d95094c1501175869adb10)) - yuxuetr
- resolve cargo-deny advisories by updating dependencies and ignoring unmaintained transitive crates - ([8ff0b56](https://github.com/yuxuetr/rust-template/commit/8ff0b564434b61ec50c0667b7a431b5cac0c06d4)) - yuxuetr
- resolve compilation error and apply formatting for JINA_API_KEY support - ([899c735](https://github.com/yuxuetr/rust-template/commit/899c7353711dc7fd3ed513bcf6da5fded5b727fc)) - yuxuetr
- correct GLM web search API endpoint and payload - ([9970faf](https://github.com/yuxuetr/rust-template/commit/9970faf1a9b2ec6a8c1a5058f1413957dac5036a)) - yuxuetr
- add dedicated glm_sensor with search_pro support and fix 401 error - ([fb1ed00](https://github.com/yuxuetr/rust-template/commit/fb1ed007bc59d7ec4096756938431f20cf87fffc)) - yuxuetr
- update GLM search response parsing to use search_result field - ([3c53271](https://github.com/yuxuetr/rust-template/commit/3c532717cd9f0be945b6578db0589852e11130c8)) - yuxuetr

### Documentation

- **(architecture)** update module layout for main.rs split - ([2ebf300](https://github.com/yuxuetr/rust-template/commit/2ebf3003ba87aa728969fe07ca3a3fc81a571dad)) - yuxuetr
- **(changelog)** regenerate for stages 13-17 - ([f26f34c](https://github.com/yuxuetr/rust-template/commit/f26f34c139778c7300e6df9e9d00fb4e9ff07831)) - yuxuetr
- **(readme)** document stages 13-17 harness features - ([5304819](https://github.com/yuxuetr/rust-template/commit/5304819b8295e99ae1db63ce2a1d452d0a979907)) - yuxuetr
- **(todos)** mark stage 13 (L1 runtime correction + L2 concurrency) complete - ([679bb67](https://github.com/yuxuetr/rust-template/commit/679bb67c87291ab6ec385daf9caba40342f49134)) - yuxuetr
- **(todos)** mark stage 14 (L4 memory deepening) complete - ([fb8dfbc](https://github.com/yuxuetr/rust-template/commit/fb8dfbcdcba66198e70ce58d83250169ef0724d7)) - yuxuetr
- **(todos)** mark stage 15 (Plan Mode / externalized state) complete - ([db7b40e](https://github.com/yuxuetr/rust-template/commit/db7b40e9f9894aa32e346cac4e8ef71d0f9eee64)) - yuxuetr
- **(todos)** mark stage 16 (three-state command policy) complete - ([d27da39](https://github.com/yuxuetr/rust-template/commit/d27da396f85ce88528e5ab4acd50c6d57802b2f2)) - yuxuetr
- **(todos)** mark stage 17.1 (Cost Tracker) complete - ([5675de7](https://github.com/yuxuetr/rust-template/commit/5675de7fcb9038490e76a7361a4950f69053a176)) - yuxuetr
- **(todos)** mark stage 17.2 (decision-path tracing) complete - ([f183bbe](https://github.com/yuxuetr/rust-template/commit/f183bbee11f5288e32bfda09d93089e0977dec2d)) - yuxuetr
- update README with multimodal sensors, @ syntax, and dual execution modes - ([46d8488](https://github.com/yuxuetr/rust-template/commit/46d84884e080b9c32d59e1414a9852e4e971a692)) - yuxuetr
- update TODOs.md to mark completed milestones - ([ac6202d](https://github.com/yuxuetr/rust-template/commit/ac6202dce8f96704c7dd5287b7adab68533c5d7e)) - yuxuetr
- mark Phase 5 tasks as completed - ([4796547](https://github.com/yuxuetr/rust-template/commit/4796547d9357eb34050638d0a55fcb20410daae6)) - yuxuetr
- mark stage 17 (L7 observability) and figure-3 checklist complete - ([ea3c9e7](https://github.com/yuxuetr/rust-template/commit/ea3c9e7b5a64119401a454437562aae3c8db3897)) - yuxuetr
- document dual OpenAI/Anthropic provider support - ([4bdb848](https://github.com/yuxuetr/rust-template/commit/4bdb8480839e5da2afe707bba58bc25efdff60f6)) - yuxuetr
- README + CHANGELOG for dual provider & reasoning handling - ([4ce372b](https://github.com/yuxuetr/rust-template/commit/4ce372b7fb1b7110b527b82a91eae90de7f5c338)) - yuxuetr
- document edit_file tool - ([00c99ca](https://github.com/yuxuetr/rust-template/commit/00c99cab1e718eef15baecf2695b9aeac6021654)) - yuxuetr

### Features

- **(agent)** context-aware Error Recovery hints + dispatcher BAD ARGS fix - ([fbebc58](https://github.com/yuxuetr/rust-template/commit/fbebc583632c51a7d5fbf07217c265702b8db0d4)) - yuxuetr
- **(agent)** System Reminders to break tool-call doom loops - ([07f2afb](https://github.com/yuxuetr/rust-template/commit/07f2afb6e62e7602db12e6668dd8cfe17a009b76)) - yuxuetr
- **(agent)** Fork-Join — read-concurrent, write-serial tool execution - ([651696c](https://github.com/yuxuetr/rust-template/commit/651696cb0fb889922861e4ec846358f10dbb6a04)) - yuxuetr
- **(agent)** dynamic Two-Stage ReAct planning phase - ([be5b0b0](https://github.com/yuxuetr/rust-template/commit/be5b0b04cbbc726404fee436cfc2b0e64247b0d9)) - yuxuetr
- **(agent)** Plan Mode — externalize long-task state to PLAN.md/TODO.md - ([0d0eaf1](https://github.com/yuxuetr/rust-template/commit/0d0eaf1d848b89adde52d72798795551c431a563)) - yuxuetr
- **(api)** Anthropic-compatible provider (DeepSeek /anthropic) - ([7444fd4](https://github.com/yuxuetr/rust-template/commit/7444fd4e3b80791e12a5580181a50c4b12583bd1)) - yuxuetr
- **(cost)** persist per-session cost to session JSON; scope to session - ([2741d2a](https://github.com/yuxuetr/rust-template/commit/2741d2ae25a3da3c140a3aea8c7aacdfe966772c)) - yuxuetr
- **(memory)** staged-degradation context compression - ([440d659](https://github.com/yuxuetr/rust-template/commit/440d65973d25190479e7738b989b17af38b4b0e9)) - yuxuetr
- **(observability)** CostTracker — session token/CNY accounting (17.1) - ([d2380a0](https://github.com/yuxuetr/rust-template/commit/d2380a06924174e1d6bcbd820ee111163e8be94c)) - yuxuetr
- **(observability)** decision-path tracing to .claw/traces (17.2) - ([c32c6ea](https://github.com/yuxuetr/rust-template/commit/c32c6eaa1a0a3b19c9f6f190b8125f587825fe64)) - yuxuetr
- **(prompt)** dynamic Prompt Composer reads workspace AGENTS.md/CLAUDE.md - ([4cec5e2](https://github.com/yuxuetr/rust-template/commit/4cec5e293c26afd2ae889f209c9c4aa975180025)) - yuxuetr
- **(security)** three-state allow/ask/deny command policy - ([1b18b64](https://github.com/yuxuetr/rust-template/commit/1b18b64465f95db54543d98e0cfaaa368b2f61eb)) - yuxuetr
- **(tools)** offload oversized tool output to temp file with preview - ([5aa4abc](https://github.com/yuxuetr/rust-template/commit/5aa4abcea6150cd045466710471375350f91ac80)) - yuxuetr
- **(tools)** add edit_file with L1-L4 fuzzy-match chain - ([209cb76](https://github.com/yuxuetr/rust-template/commit/209cb7684764b7c37f60bcad5079a823db5e6a51)) - yuxuetr
- implement multimodal API and Sensor-Brain vision pipeline - ([d6fe6c7](https://github.com/yuxuetr/rust-template/commit/d6fe6c7ae57775c47c99e40c80282a458afbdf61)) - yuxuetr
- integrate MinerU V4 API for PDF parsing and add configuration management - ([462e74a](https://github.com/yuxuetr/rust-template/commit/462e74af95a1f20287a8f3ed74af04b23283afa7)) - yuxuetr
- unify sensor data integration with @paste syntax and refactor paste_image - ([146613e](https://github.com/yuxuetr/rust-template/commit/146613eb3c400638eaa17bc5acec35ad8a07947b)) - yuxuetr
- add @image and @img as keywords for clipboard image analysis - ([f65a93d](https://github.com/yuxuetr/rust-template/commit/f65a93d15d065b7de30318d9feb3008f7d5e5928)) - yuxuetr
- rename /paste to /image and add /file subcommand for standalone analysis - ([13a8f5a](https://github.com/yuxuetr/rust-template/commit/13a8f5a21ea17f7aa54b9b98bf1efd472e9d4644)) - yuxuetr
- make /image and /file standalone tools that do not affect conversation history - ([b01c2b7](https://github.com/yuxuetr/rust-template/commit/b01c2b7f3d2ac73a901142e7806f51aa3dc72f67)) - yuxuetr
- support quoted paths and spaces in commands and @ references - ([4ff5b39](https://github.com/yuxuetr/rust-template/commit/4ff5b395584b1ef51acad6caa59bc8396a26859f)) - yuxuetr
- implement Web Sense with @url and /web command using Jina Reader - ([9b68923](https://github.com/yuxuetr/rust-template/commit/9b68923bc5734cda65da73ffbe0bc303b3c2472e)) - yuxuetr
- integrate GLM and Tavily search sensors with standalone and inline modes - ([dfd511b](https://github.com/yuxuetr/rust-template/commit/dfd511bd2e7ddd953032c3b228fa37efa6dd334a)) - yuxuetr
- expose subcommands to top-level CLI for direct shell usage - ([eaab7a3](https://github.com/yuxuetr/rust-template/commit/eaab7a38475ae1d3babc3155e9282fcbd179d4c5)) - yuxuetr
- implement tool dispatcher and core system tools (fs, shell) - ([3ac3882](https://github.com/yuxuetr/rust-template/commit/3ac3882dd7228bf5cfdb41069d3f797b46a84ef1)) - yuxuetr
- implement ReAct loop in core chat engine - ([d439aa5](https://github.com/yuxuetr/rust-template/commit/d439aa5167c8025ce732c99d6516511498f60483)) - yuxuetr
- implement Sub-Agent delegation mechanism - ([b94c933](https://github.com/yuxuetr/rust-template/commit/b94c93379306eb4d56aca223a0a7fc2330f6c990)) - yuxuetr
- implement Dynamic Skill Generation mechanism - ([ffdb784](https://github.com/yuxuetr/rust-template/commit/ffdb78456e3c3e7bf5f31089b7c3fa15f2b3bb82)) - yuxuetr

### Other

- **(ci)** update version for checkout and git-cliff - ([c3f1771](https://github.com/yuxuetr/rust-template/commit/c3f1771ee3f613df0217d61767108295e41b393d)) - yuxuetr
- **(ci)** update version for checkout and git-cliff again - ([bc1acfd](https://github.com/yuxuetr/rust-template/commit/bc1acfd287dd1dd46c7367fdcffa475e2a06d18c)) - yuxuetr
- update github actions build workflow rust setup configuration - ([41f35a2](https://github.com/yuxuetr/rust-template/commit/41f35a28931e8139c7d8002d015190856e2ce3c4)) - yuxuetr

### Refactoring

- **(api)** introduce LlmProvider trait; OpenAI client behind it - ([20f4302](https://github.com/yuxuetr/rust-template/commit/20f4302cffbb6a65823ce044a075c1a17faacc7c)) - yuxuetr
- **(main)** extract rustyline completer to completer.rs - ([9c08ecf](https://github.com/yuxuetr/rust-template/commit/9c08ecfc7d8bfab2dd185b197cbc0286078b98f8)) - yuxuetr
- **(main)** extract run_benchmark to benchmark.rs - ([346bf20](https://github.com/yuxuetr/rust-template/commit/346bf20a5627ed3a98b2648ebe794bd078841df5)) - yuxuetr
- **(main)** extract ReAct engine to engine.rs - ([6a473ca](https://github.com/yuxuetr/rust-template/commit/6a473ca74d5d155f73e3df5f0d88bfcd8c0c38db)) - yuxuetr
- **(main)** extract REPL command handling to commands.rs - ([71b7f44](https://github.com/yuxuetr/rust-template/commit/71b7f44e01e247283244f774a854e35fc22c7bf1)) - yuxuetr
- **(observability)** store traces under ~/.seekcli/traces - ([4bab2cb](https://github.com/yuxuetr/rust-template/commit/4bab2cb0d8656fb86274e98a974e3ba2a55e4ff5)) - yuxuetr
- remove arboard and use system commands for clipboard, revert to multi-threaded tokio - ([1d77a42](https://github.com/yuxuetr/rust-template/commit/1d77a42e07e4472bf5d4764534f0403e6bf1f28e)) - yuxuetr

### Tests

- fix test_load_all_skills by asserting only default skills - ([2b8443e](https://github.com/yuxuetr/rust-template/commit/2b8443e4410832411c87a9a0c01d82bd66fbf160)) - yuxuetr

---
## [0.1.0] - 2026-04-29

### Documentation

- fix README.md and add CHANGELOG.md - ([d354109](https://github.com/yuxuetr/rust-template/commit/d3541095ee693fe1d8ccb1e4b889f055a381ad2a)) - yuxuetr
- update README and CHANGELOG, add development roadmap v0.1.0 - ([ac5bf82](https://github.com/yuxuetr/rust-template/commit/ac5bf8212c7052df8a6042a87026257683dfc50f)) - yuxuetr

### Features

- support deepseek chat and r1 model for cli - ([200af21](https://github.com/yuxuetr/rust-template/commit/200af215b4f6973871bc105f16210946e4316392)) - yuxuetr
- using dashscope api and support stream mode - ([c44c732](https://github.com/yuxuetr/rust-template/commit/c44c732ff73b440c7b24188c16dde06caf310ba3)) - yuxuetr
- add markdown rendering support via termimad and improve output display - ([8f7568f](https://github.com/yuxuetr/rust-template/commit/8f7568f883f200303cc6a22d8a3a1de089bdd9d3)) - yuxuetr
- implement syntax highlighting, code block extraction, and /copy command - ([6fd0d0a](https://github.com/yuxuetr/rust-template/commit/6fd0d0a086f469d4094be3935ecd73943dd16023)) - yuxuetr

<!-- generated by git-cliff -->
