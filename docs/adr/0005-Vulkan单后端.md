# ADR 0005：Vulkan 唯一图形后端

- 日期：2026-11-18
- 状态：Superseded by ADR 0014（wgpu 首选，Vulkan 保留为回退/显式后端；原文保留作历史）
- 关联：README §5.1 规则 2、§9.6、§11.6、RFC 0001

## 背景

RHI 多后端（DX12/Vulkan/Metal）是引擎复杂度的核心来源。作为新项目，需要在起步阶段决定后端矩阵，
以控制工程量并兼顾 Windows 发布与 Linux 接口保留的目标。

## 决策

1. **唯一图形后端：Vulkan 1.3，通过 `vulkanalia` 0.35**。禁止引入 `wgpu`、`ash`、DX12、OpenGL、Metal；
   不做第二套 RHI 抽象。
2. **封装边界**：Vulkan 类型只出现在 `mir-render` 内部，运行时/编辑器公共 API 不得暴露；
   平台层只通过 `raw-window-handle` 0.6 传递窗口句柄（`mir-window::Window::raw_handle()`）。
3. **跨平台策略**：Vulkan 与 winit 本身跨平台；Linux 启用时仅替换 surface/平台后端，渲染与玩法代码不变。
   当前 Windows 是唯一发布平台（详见 ADR 0007）。
4. **运行前提**：Windows 10/11 x64 + 支持 Vulkan 1.3 的驱动；启动时检测 API/特性并给出可读错误，
   **不提供 DX12 兜底**。
5. **依赖版本钉定**：`vulkanalia` 必须启用 `window` feature（提供 `vulkanalia::window` 与 KHR 扩展命令），
   其 rwh 0.6 与 `winit` 0.30 兼容（RFC 0001 实测确认）。
6. 许可证：`vulkanalia` 为 Apache-2.0，与项目 MIT OR Apache-2.0 双许可兼容（§11.6）。

## 替代方案

- **多后端 RHI（DX12 + Vulkan）**：覆盖面广，但需维护两套命令/资源抽象与测试矩阵，工程量翻倍，不适合早期项目。
- **wgpu**：跨平台易用，但抽象层屏蔽了显式同步/显存控制，与 §9.6 的 dynamic rendering / synchronization2 规划冲突）。
- **ash**：更底层，需手写 loader 与更多样板代码；vulkanalia 的封装更适合本项目。
- **DX12 唯一**：绑定 Windows，失去与 Linux 接口对齐的可能。

## 后果

- 正面：API 面收敛，M2 能集中资源做深（RenderGraph、显存、缓存）；一台集成显卡开发机即可满足（RFC 0001 实测 Intel Iris Xe 通过）。
- 负面：无 DX12 兜底，老旧无 Vulkan 1.3 的机器无法运行；驱动兼容性需矩阵测试。
- 风险：M0 PoC 已验证核心链路无阻塞；剩余风险（validation layer、交换链重建）在 M2 处理。
