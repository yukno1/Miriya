# ADR 0004：渲染管线与 RenderGraph

- 日期：2026-11-18
- 状态：Accepted（着色器语言部分由 ADR 0011 取代，其余决策仍然有效）
- 关联：README §9.6、RFC 0001

## 背景

需要确定 M2 起的渲染架构：如何组织 Vulkan pass、barrier、资源生命周期与着色器编译，
同时保持渲染层与逻辑/ECS 解耦，并支撑后续 PBR/IBL 与后处理扩展。

## 决策

1. **Clustered Forward+ 管线**，顺序：CSM 阴影 → 深度预通道 → 不透明（PBR + IBL）→ 透明 → SSAO/Bloom/色调映射 → UI。
   不做 Lumen/Nanite 等价；GI/虚拟几何列入 N3 研究。
2. **RenderGraph**：每个 pass 声明读写资源，由 RenderGraph 自动推导 barrier 与 layout 转换、
   临时纹理别名、GPU 时间戳；每帧可 dump 依赖图用于调试。
   > 追记（2026-11-24）：v0.5 已随 M2 批次 B 落地——声明式 pass/资源（graphics/transfer）、
   > Kahn 拓扑排序 + 逐资源布局追踪自动生成 synchronization2 barrier、transient 别名、
   > `vkCmdWriteTimestamp2` pass 耗时进 `FrameStats`、`--graph-dump` 输出 DOT；
   > v0.2 的 3 处每帧手写 barrier 已全部由图编译器接管。
3. **数据解耦**：`extract` 阶段把 ECS 转成纯帧数据（相机、光源、实例、材质），`mir-render` 不访问 ECS、不依赖 `mir-scene`。
4. **Vulkan 1.3 特性**：dynamic rendering、synchronization2、timeline semaphore、descriptor indexing；
   自研显存子分配器（块/池/延迟回收），不引入 `ash`/VMA 之外的第三方 RHI。
   > 追记（2026-11-24）：dynamic rendering / synchronization2 已在 `Vulkan13Features` 探测并启用；
   > 自研子分配器随 M2 批次 E 落地（memory type 分桶、块分级、空闲合并、dedicated 整块、
   > `DeletionQueue` 绑定帧 fence 延迟回收）。timeline semaphore / descriptor indexing
   > 仍按需评估（m2-remaining.md §8）。
5. **着色器**：Slang（公共 module + `import`）经 `slangc` 直接编译为 SPIR-V，阶段入口统一为 `main`；
   `spirv-reflect` 生成绑定；`spirv-val` + validation layer 把关；离线缓存以管线哈希为键，开发模式热重载。
6. **帧结构**：2–3 帧在飞行、环形暂存缓冲；交换链重建/设备丢失有恢复路径；调试版启用 Khronos validation layer，
   发布版持久化 pipeline cache。
   > 追记（2026-11-24）：帧在飞行取 2 并已落地；管线缓存（`VkPipelineCache` 常驻 + 磁盘持久化，
   键 = 设备 UUID + 驱动版本 + 管线状态哈希）随 M2 批次 D 完成。
   > 追记（2026-11-24，批次 F）：材质参数走 set 1 per-material UBO（std140 48 B，dedicated 分配），
   相机 UBO 扩为 80 B（增加相机世界位置供 PBR 高光）；PBR 首版为解析式 GGX metallic-roughness
   + 单方向光 + 固定半球环境项（IBL/CSM/SSAO/Bloom 归 M5）；视锥剔除在应用层 extract 侧
   （`mir-math::Frustum`/`Aabb`），渲染层不感知剔除（决策 3 的延伸）。
7. **前提已验证**：M0 PoC（RFC 0001）确认实例/设备/surface/交换链前置查询链路可用。

## 替代方案

- **Deferred / 传统 Forward**：前者对透明与 MSAA 处理复杂，后者灯光扩展性差；Clustered Forward+ 兼顾二者。
- **手动管理 barrier / 无 RenderGraph**：短期直接，长期资源转换易出错，无法支撑多 pass 后处理。
- **引入第三方 RenderGraph 库**：与自研渲染节奏不合，且增加公共 API 面积。
- **SPIR-V 直接手写 / 用其他着色语言**：GLSL 生态成熟但模块化弱；改用 Slang 以获得 module/import 与跨 API 代码生成能力（见 ADR 0011）。

## 后果

- 正面：pass 扩展与 barrier 正确性由结构保证；渲染与逻辑独立演进；热重载与缓存改善迭代速度。
- 负面：RenderGraph 与显存子分配器是 M2 内的重要工程量；需持续跟踪 1080p 60fps / 1M 三角形 / GPU ≤ 16.6 ms 的性能预算。
- 准入：M2 验收要求 v0.2「旋转立方体 + 自由相机」1080p 稳定、validation layer 无错误。
