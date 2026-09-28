---
kind: plan
---

# 实际使用问题审计（2026-09-28）

本页记录一次面向"实际投入使用"的审计结论，供后续修复参照。每条问题修复后请删除对应条目；全部处理完毕后删除本文件及 [文档索引](../README.md) 中的链接。

审计对象：`main` 分支 `82889b9`（已发布版本 v0.1.0 = `cc93e11`）。

方法：通读 README / docs / 规则文档与 `src/` 核心实现；本地 `cargo build --release` 后，在一个模拟的"普通开源项目"（含 README、CHANGELOG、CONTRIBUTING、`.github/` 模板、`docs/`、`node_modules/`、docs 站点风格链接、中英混排文档）里实际跑 `init` / `check` / `hook` / `--fix` / `--stdin-filename` 等命令，对照文档逐条验证。`cargo test --locked` 全部通过。

---

## 一、高优先级：会直接影响实际使用价值或造成误导

### 1. 没有适合社区文件和模板的 kind

- 8 个 kind 里没有适合 CONTRIBUTING / CODE_OF_CONDUCT / SECURITY / issue & PR 模板 / 许可说明的类型。roadmap 里 "Whether projects need custom kinds" 仍是 open question。
- `seiso init` 目前的缓解办法：把 CONTRIBUTING / SECURITY / SUPPORT 映射为 `howto`，把 issue / discussion / PR 模板和 CODE_OF_CONDUCT 加入 `exclude`。其他没有合适 kind 的文件（如 `docs/api/index.md`）仍会报 `KND001`。
- 建议：决定是否增加一个不受 kind 约束的通用 kind（如 `misc`/`other`），或把 `KND001` 改成只在文件匹配到 `[[kinds]]` 范围之外时才报。

### 2. `LNK001` 不做扩展名/index 补全，大小写结果随平台变化

- 纯物理路径解析（`src/paths.rs::local_link_target` + `fs::metadata`）。`[Setup](docs/setup)`（文件是 `docs/setup.md`）和以站点根为基准的 `/site/docs/setup` 都会报 LNK001。LNK001 规则文档已写明这些语义，并建议用 `per-file-ignores` 处理按站点路由写链接的目录。
- `DOCS/setup.md`（大小写错误）在 Windows / macOS 上通过、Linux CI 上会失败。规则文档已说明，但结果仍取决于平台。
- 建议：评估是否提供 `lint.lnk.resolve-extensions` 之类选项；评估在大小写不敏感的文件系统上也按目录项做精确大小写比对。

---

## 二、中优先级：逻辑/文案不一致或体验问题

### 配置与命令

1. **父目录已有配置时 `init` 直接报错**，无法为子目录建独立配置。
2. **配置层面没有 `extend-select` / `extend-ignore`**：`extend` 继承时数组整体替换（已写入 configuration.md），子配置想在父配置的 `lint.select` 上追加规则只能整份重写；追加只有 CLI `--extend-select`。

### 规则与诊断

3. **STL001 与 STL004 对同一位置双报**：`目前版本是 v1.2.3` 同一 span 同时报 STL001 和 STL004，两条建议内容近似。
4. **STL001 的英文 stale 词表不含 "current"**：只有 `currently` / `latest` / `at present`。README 首段举的例子 "a stale version number" 最常见写法 `Current version: 1.2.3` 靠 STL004 才能命中。
5. **EVD001 把 "we recommend" / "is recommended" 视为需证据的评价**：how-to 里 "We recommend X" 是正常写法；VOX003 的 narration 词表含 "this document was generated"——这恰恰是很多 README 用来提醒读者"别手改"的合法声明。（都是 preview，但词表设计值得复查。）
6. **JSON 输出 `url` 字段恒为 null**：规则文档在 GitHub 有稳定 URL，可以填。

### 分发与集成

7. **npm 包没有 macOS / arm64 / musl 二进制**：`package.json` 声明 `"os": ["linux","win32"], "cpu": ["x64"]`，其他平台 npm 以 EBADPLATFORM 拒绝安装，没有源码回退（README 已改为如实说明）。开发者工具缺 macOS 二进制是显著缺口；Alpine（musl）上 linux-x64 glibc 二进制也起不来，`bin/seiso.cjs` 的报错只会说 "cannot start the bundled binary"。
8. **pre-commit hook 需要 Rust 工具链**（`language: python` → maturin 源码构建），文档有写；但 `.pre-commit-hooks.yaml` 没有 `minimum_pre_commit_version`，integrations.md 用 `<reviewed-revision>` 占位而不直接给 `v0.1.0`。

### 版本与文档漂移

9. `main` 上有 9 条 v0.1.0 之后新增的规则（18 → 27）及 `sections.md`、`sections.rs`，`Cargo.toml` / `package.json` 版本仍是 `0.1.0`。用户按 GitHub main 文档 `seiso rule STL002` 会得到 "not implemented"。roadmap 已把 M3 写成 delivered。
10. `docs/guides/development.md` 和 CI 用 `cargo run -p seiso`，但这是单包项目，`-p seiso` 冗余；工作区里还残留空的 `crates/seiso_*` 目录（未跟踪，本地清理即可）。
11. `docs/reference/configuration.md` 说 "Rule documentation identifies each rule's thresholds and word-list options"，但只有部分规则文档真的写了阈值/词表；`lint.dup.*` 五个字段的默认值只能去读 `src/config/mod.rs`。

---

## 三、低优先级 / 措辞

1. README 开篇 "Hardly anyone writes project docs by hand anymore. AI writes most of them" 是无依据的断言——项目自己的 EVD001 就是针对这类 "evaluative claims without evidence"。可以改成更克制的表述。
2. README 用 rustfmt 类比 "without agreeing on them first"，但 seiso 实际要求每个项目先配置 kind 映射、给每篇文档定 kind，配置负担和 rustfmt 的零配置正相反。类比容易让人预期落空。
3. README 说 "Each diagnostic says where the problem is and how to fix it, so an agent can repair the page from seiso's output alone"——对 KND/LNK/SUP 成立；对 DUP/OWN 的建议是 "choose one authoritative document"，agent 无法仅凭输出决定。
4. integrations.md Claude Code 一节的 matcher `"Write|Edit"` 是正则，会同时匹配 `MultiEdit`（对 seiso 无害，但读者可能以为只匹配两个工具）。
5. Text 渲染里每条诊断都带源码摘录，一个 300 篇文档的仓库首次 `check` 会输出上千行 KND001；concise 更适合作为默认，或至少在 KND001 大量出现时折叠。

---

## 四、核验为正确/做得好的部分（避免只报问题）

- 文档对实现的描述总体准确：配置发现顺序、`last wins`、`--select` 替换 / `--extend-select` 追加、exit code 0/1/2、`--exit-zero` 保留 2、stdin 覆盖、`--fix` 的二次校验与写前源比对、hook exit 0/1/2 映射、`policy` 的 `not_evaluated` 语义，均与实测一致。
- CRLF、UTF-8 BOM、Windows 反斜杠路径、从子目录运行、路径不存在、非法 selector 等边界情况处理正确，错误信息清楚。
- `--fix` 只删确认无效的 suppression code、保留原因和其它 code、保持 CRLF；写入前比对源文件；拒绝 symlink / 只读文件。
- 评估记录（`docs/evaluation/*`）对自身局限（agent 标注、样本为零、精度不可用）的陈述非常诚实；release notes 对 stable/preview 边界的说明清楚。
- CI 结构合理（docs-only 分流、Linux+Windows 测试、`check` 汇总 job）；`cargo test --locked` 本地全部通过。
- 输出格式（text/concise/json/sarif/github）齐全，GitHub 输出对 `##[` 注入做了防护。

---

## 五、建议的处理顺序

1. 决定社区文件与模板的 kind 方案（第一部分 1）。
2. 评估 LNK001 的扩展名补全选项与跨平台大小写一致性（第一部分 2）。
3. 发布包含 M3 规则的新版本，消除 main 文档与已发布版本的漂移（第二部分 9）。
4. 增补 macOS / musl 二进制，或让 npm 在不支持的平台给出明确指引（第二部分 7）。
5. 复查 preview 规则的双报与词表问题（第二部分 3-5）。
