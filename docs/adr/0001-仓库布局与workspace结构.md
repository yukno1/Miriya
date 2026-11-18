# ADR 0001：仓库布局与 workspace 结构

- 日期：2026-11-18
- 状态：Accepted
- 决策者：Miriya 项目
- 关联：README §6

## 背景

需要确定 Cargo workspace 的组织方式：按引擎/编辑器/工具分包，并选择扁平还是嵌套目录。
项目需用同一套 crate 支撑运行时、编辑器、CLI 工具与 C# 脚本层，且依赖方向必须可静态检查。

## 决策

采用**扁平 workspace**：单一 `Cargo.toml`，`members = ["crates/*", "tools/*"]`，
共 40 个成员（37 个 crates + 3 个 tools）。命名规则：

- `mir-*`：引擎内部 crate（`mir-core`、`mir-render`、`mir-ecs` …）
- `miriya-*`：门面、应用与工具（`miriya-engine`、`miriya-app`、`miriya-cli` …）
- `tools/`：开发/构建期工具（`miriya-cli`、`mir-build`、`mir-bindgen`），M0 新增 `mir-vulkan-probe` spike
- `managed/`：C# 项目（`Miriya.Runtime`、`Miriya.Scripting`）
- `docs/`：`adr/`（本目录）、`rfcs/`（大特性提案）、`book/`（mdbook）

目录结构对齐 README §6 的目标树；库 crate 即使当前为空壳也先建好，均纳入 workspace 编译检查。
workspace 设 `default-members = ["crates/miriya-app"]`，使根目录 `cargo run` 直接进入示例。

依赖方向（README §5.1）：应用层 → 门面 → 编辑器/工具 → 运行时 → 数据层 → 平台层 → 基础层；
底层不得依赖上层，子 crate 必须显式声明依赖，禁止借 facade 的 re-export 形成隐式耦合。

## 替代方案

- **嵌套 workspace**（`crates/engine/*`、`crates/editor/*` 多 workspace）：能分组，但增加成员路径层级与 CI 配置复杂度；扁平结构配合命名前缀已足够表达分层。
- **单 crate + 模块**：无法做到按 crate 的编译隔离与依赖检查，编辑器/UI 依赖会污染运行时。
- **多个独立 workspace**：跨 crate 引用需路径或版本发布，开发与 CI 成本更高。

## 后果

- 正面：依赖方向可用 `cargo tree` / CI 脚本直接审查；空壳 crate 参与编译，接口腐烂会尽早暴露；新手能从目录名判断 crate 角色。
- 负面：40 个成员使 `cargo check --workspace` 输出较长；扁平目录需靠命名规范维持可读性，需在评审中强制执行。
- 后续：新增 crate 必须同步更新 README §6 状态表，并确认其依赖方向符合分层。
