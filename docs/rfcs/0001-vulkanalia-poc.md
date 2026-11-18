# RFC 0001：vulkanalia 最小 PoC 风险结论

- 日期：2026-11-18
- 状态：已完成（Accepted）
- 关联：M0 任务「vulkanalia / .NET 8 hostfxr 最小 PoC」的 Vulkan 部分；ADR-0005

## 摘要

在 M0 阶段用独立 spike 工具 `tools/mir-vulkan-probe` 验证 M2 渲染链路的最大技术风险：
`vulkanalia` 能否与现有 `winit` 窗口 + `raw-window-handle` 0.6 协同，完成 Vulkan 1.3 的
实例 → 物理设备 → 逻辑设备 → surface → 交换链前置查询。结论：**链路全部打通，M2 无阻塞性风险**。

## 背景

README §8 把 M2（Vulkan 渲染 v0）列为 12 人周的关键里程碑，依赖三个尚未验证的假设：

1. `vulkanalia` 0.35 的 API 能用稳定 Rust（本机 1.100.0）编译；
2. 它与 `winit` 0.30 使用的 `raw-window-handle` 0.6.2 版本兼容；
3. 目标开发机具备可用的 Vulkan 1.3 设备，且 surface/队列族选择链路可行。

任一假设不成立都会让 M2 的排期失效。故在 M0 先做 spike，不污染运行时 crate。

## 验证方法

新增 `tools/mir-vulkan-probe`（仅 PoC，不进入 `mir-render`），完整链路：

```
LibloadingLoader(vulkan-1.dll) → Entry → Instance(1.3, KHR_surface + 平台 surface 扩展)
→ winit 窗口 → vulkanalia::window::create_surface（rwh 0.6 桥接）
→ enumerate_physical_devices → 选 GRAPHICS + present 队列族
→ surface formats / present modes → create_device(VK_KHR_swapchain)
```

命令：`cargo run -p mir-vulkan-probe`，运行后短暂打开窗口并输出探测报告。

## 结果（本机实测）

| 项目 | 结果 |
|---|---|
| Loader 加载 | ✅ `vulkan-1.dll`，版本 1.4.341 |
| Instance 创建 | ✅ Vulkan 1.3 API |
| Surface 创建 | ✅ rwh 0.6 桥接正常（Win32） |
| 物理设备 | Intel(R) Iris(R) Xe Graphics |
| Vulkan 1.3 支持 | ✅ |
| 图形队列 / 呈现队列 | ✅ / ✅ |
| Surface 格式 | B8G8R8A8_UNORM |
| 呈现模式 | IMMEDIATE |

## 关键发现与决策

1. **版本兼容已确认**：`vulkanalia` 0.35 的 `window` feature 直接依赖 `raw-window-handle` 0.6，
   与 `winit` 0.30 完全一致，无需适配层。
2. **`window` feature 是必需的**：只启用 `raw-window-handle` 不会导出 `vulkanalia::window` 模块，
   也不会启用 `Khr*ExtensionInstanceCommands` 扩展命令 trait。M2 的 `mir-render` 必须启用 `features = ["window"]`。
3. **unsafe 边界清晰**：loader 加载、实例/设备创建、表面创建、设备枚举均为 unsafe，
   与 README §11.2「unsafe 集中在 FFI/Vulkan 边界并附 SAFETY 注释」一致。
4. **无需 DX12/OpenGL 兜底**：本机仅有集成显卡即可满足，支持 README §9.6「唯一 Vulkan 后端」决策。
5. **设备选择逻辑可复用**：GRAPHICS 位 + `get_physical_device_surface_support_khr` 选择队列族的模式
   可直接搬入 M2 的 `mir-render`。

## 对 M2 的影响与后续

- **风险结论**：M2 无阻塞性技术风险；PoC 中验证过的调用序列作为 M2 实现参考。
- **M2 仍需自行验证**：validation layer、dynamic rendering、synchronization2、交换链重建、显存子分配，
  这些不在本 PoC 范围。
- **CI 注意**：GPU 运行时测试在无硬件时允许跳过并显式标注（README §11.3），本 PoC 是手动 spike 工具，
  不加入 `cargo test`；其编译纳入 workspace `cargo check`。
- **工具去留**：`mir-vulkan-probe` 保留为诊断工具（类似 `miriya doctor`），M2 完成后可进化为真实的
  `miriya doctor --vulkan` 子命令。

## 替代方案

- 用 `vulkanalia-bootstrap` 简化初始化：放弃，其封装过度且不利于 M2 精细控制设备/队列选择。
- 用 `ash` + 手写 loader：与 ADR-0005「唯一 vulkanalia 后端」冲突，不采用。
- 跳过 PoC 直接做 M2：风险过高，一旦 rwh 版本不匹配将浪费整个 M2 前期工作。
