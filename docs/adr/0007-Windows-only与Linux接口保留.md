# ADR 0007：Windows-only 实现 / Linux 接口保留

- 日期：2026-11-18
- 状态：Accepted
- 关联：README §5.5、§11.1

## 背景

需要在起步阶段收窄平台矩阵以降低工程量，同时避免把公共 API 与 Windows 专有类型绑死，
为后续 Linux 支持保留可能。目标平台曾计划多平台，现收敛为 Windows 优先。

## 决策

1. **Windows x64 是唯一可发布实现**：完整平台后端 + Vulkan 1.3。
2. **Linux x64 保留接口**：平台能力（窗口、文件系统、计时、线程策略、音频设备、崩溃上报）以 trait 定义，
   Linux 提供占位实现（返回 `Unsupported` 或明确错误），并纳入 CI 编译检查，防止接口被 Windows 专有类型污染。
3. **公共 API 只暴露跨平台抽象与 `raw-window-handle`**：不得出现 `HWND`、`HINSTANCE` 等 Windows 专有句柄。
4. **CI 策略**：Windows 跑 fmt/clippy/test + .NET 构建；Linux 跑 `cargo check --target x86_64-unknown-linux-gnu`
   （只验证接口，不跑运行时）。Linux CI 需安装 winit 的 X11/Wayland 系统依赖。
5. **渲染/脚本平台无关**：Vulkan 与 winit 本身跨平台；Linux 启用时只替换 surface/平台后端与路径策略，
   `mir-render`、`mir-script` 与玩法代码不改。
6. **发布承诺**：Linux 运行时验收、打包与支持属于环 2（N6），当前只发布接口编译检查结果；
   macOS / 移动端 / 主机不维护。

## 替代方案

- **多平台首发**：Windows/Linux/macOS 同时实现，平台矩阵与测试成本远超早期团队承受范围。
- **完全不提 Linux**：公共 API 容易出现 Windows 类型泄漏，未来扩展需重构。
- **用 WGPU 换跨平台**：与 ADR 0005「唯一 vulkanalia 后端」冲突。

## 后果

- 正面：工程量聚焦单一发布平台；trait 抽象 + Linux 编译检查保证公共 API 不腐烂；未来启用 Linux 只需实现平台后端。
- 负面：Linux 用户当前无法运行；CI 需维护 Linux 侧系统依赖与编译目标。
- 约束：任何在公共 API 引入 `HWND`/`HINSTANCE` 的改动视为评审阻塞项。
