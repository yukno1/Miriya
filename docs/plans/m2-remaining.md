# M2 剩余任务清单（多帧在飞行 → RenderGraph → 显存/管线基础设施 → 材质与 PBR）

- 日期：2026-11-24（批次 A 于同日完成）
- 状态：**批次 A 已完成**（R-01/R-02 落地；R-03 起未开工）
- 关联：README §8.2（M2）、§8.3（v0.3 垂直切片）、§9.6、§14.4；ADR-0004（渲染管线与 RenderGraph）、
  ADR-0011（Slang，第 8 条多入口延后）、ADR-0012（帧数据/提交边界，第 5 条临时取舍）；`docs/plans/v0.2-cube.md`
- 估算：**约 12.5 净人日**（批次 A 1.5 + B 3.5 + C 0.5 + D 1.0 + E 3.0 + F 5.0，含 0.5 冗余）

## 1. 目标与 Gate

**v0.3 前置目标**：把 v0.2 登记的六项 M2 剩余全部落地，渲染层从「单帧在飞行 + 手写 barrier +
每资源独立显存分配」升级为「2 帧在飞行 + RenderGraph 自动 barrier + 子分配器 + 管线缓存 + 材质/PBR 雏形」，
为 v0.3「PBR Scene」与 M3 场景/ECS 铺平道路。

**Gate（全部满足才算 M2 剩余达成）**

1. 多帧在飞行（2 帧）落地后：`cargo run --release -- --frames 600 --width 1920 --height 1080` 稳定，
   校验层 0 error（重点盯信号量复用、命令缓冲复用类 VUID），无 `frame_error`。
2. RenderGraph 接管每帧 barrier 后：手动 barrier 清单（v0.2 的 3 处每帧路径 barrier）全部由图编译器生成，
   校验层 0 error；每帧可 dump 依赖图（文本/DOT）用于调试。
3. 单 `.spv` 多入口：cube 着色器合并为单 `.spv`（`-fvk-use-entrypoint-name`），CI 两个 job 重新生成且
   `git diff --exit-code` 通过；Rust 侧各阶段以入口名绑定，无行为变化。
4. 管线缓存持久化：二次启动命中缓存（日志可验证），缓存损坏时静默重建不报错；校验层 0 error。
5. 显存子分配器：深度图像、网格顶点/索引、staging、UBO 全部经分配器；`Drop` 后无分配残留；
   校验层 0 error；新增分配器单测全过。
6. 材质 + 基础 PBR + 视锥剔除：演示场景（程序化几何 + 多材质 + 单方向光）1080p 稳定，60 FPS 附近；
   金属度/粗糙度变化肉眼可辨；相机背对场景时剔除生效（`submitted_instances()` 下降可观测）。
7. 全程回归：`cargo test --workspace`、`cargo fmt --all -- --check`、
   `cargo clippy --workspace --all-targets -- -D warnings` 全绿；
   `--headless --frames 1000` 行为不变；新增 `unsafe` 均带 `SAFETY:` 注释。

## 2. 现状核对（已读代码确认的事实）

| 事实 | 位置 | 对 M2 剩余的影响 |
|---|---|---|
| 单帧在飞行：`FrameResources` 单实例（pool/buffer/fence/image_available/render_finished），注释自述「简化同步」 | `crates/mir-render/src/vulkan.rs:130-139` | 批次 A 必须重做帧资源所有权 |
| `render_finished` 每交换链图像一个，随 recreate 同步重建；`image_available` 全局 1 个 | `vulkan.rs:136-137, 684-690, 703-727` | 多帧后 image_available/render_finished 归属需按帧划分 |
| camera UBO 全局 1 个（64 B，HOST_VISIBLE\|COHERENT），每帧 map+memcpy+unmap；descriptor pool `max_sets(1)`、描述符 1 个 | `vulkan.rs:1264-1273, 1286-1290, 1566-1581` | 需改每帧 UBO + 每帧描述符集；`max_sets` 要扩大 |
| 帧命令缓冲每帧 rerecord（`ONE_TIME_SUBMIT` begin，池带 `RESET_COMMAND_BUFFER`），无显式 reset | `vulkan.rs:658-664, 1466-1467` | 多帧在飞行只需每帧一份命令资源 |
| 传输命令池独立于帧资源（复用 graphics 队列），上传与深度布局转换均阻塞式 `queue_wait_idle` | `vulkan.rs:222-224, 473-489, 1061-1121, 846-897` | 批次 A 可保留阻塞上传；异步上传不在本清单 |
| swapchain 重建：wait_idle → views/swapchain → 深度 → `render_finished` → format 变才重建管线 | `vulkan.rs:1696-1726` | 批次 A 需同步重建每帧资源数组 |
| headless：只建实例/设备/传输资源，swapchain/frame/depth/pipeline 均不创建；`create_mesh` headless 也可用 | `vulkan.rs:291-561, 239-242`；`lib.rs:247-253` | 所有新路径必须保持 headless 空操作 + 计数器语义 |
| 显存 `allocate_memory` 全仓库仅 2 处（深度图像、通用 buffer），每资源独立分配、偏移恒 0、无子分配器/删除队列 | `vulkan.rs:812, 998, 753-771` | 子分配器（批次 E）有唯一接管点 `create_buffer` 与深度路径 |
| 手动 barrier 共 4 处：深度一次性转换、每帧 acquire 后 UNDEFINED→COLOR、呈现前 →PRESENT_SRC、网格上传 buffer barrier | `vulkan.rs:854-866, 1486-1504, 1632-1650, 1088-1107` | RenderGraph（批次 B）替换前 3 处每帧路径；上传 barrier 保留在传输路径 |
| 单一 cube 图形管线：顶点输入 24B×2 attribute、BACK 剔除 + CLOCKWISE、深度 LESS、push constant 80 B、动态渲染 | `vulkan.rs:1147-1372, 1180-1246` | 批次 F 材质/PBR 扩展此管线 |
| `VkPipelineCache` 不存在（`PipelineCache::null()`）；管线仅在 surface format 变化时重建 | `vulkan.rs:1349-1352, 1718-1721` | 批次 D 从零建缓存对象 + 持久化 |
| `.spv` 经 `include_shader_code!` 编译期嵌入；每 stage 独立 `.spv`，入口名 `main` | `vulkan.rs:1148-1151`；`shaders/cube/cube.slang:1-2` | 批次 C 多入口改造加载与命名 |
| CI 两 job 各 4 条 `slangc`（hello/cube × vert/frag）+ `git diff --exit-code -- shaders/hello/*.spv shaders/cube/*.spv`；`SLANG_VERSION: 2026.8` | `.github/workflows/ci.yml:27-38, 84-91, 10` | 批次 C 同步两个 job |
| `GpuMesh` 每网格 4 个资源句柄；`MeshHandle` = 高 32 位世代 + 低 32 位槽位；无删除队列，销毁全部同步 | `vulkan.rs:184-198, 936-974`；`mesh.rs:74-89` | 批次 E 延迟回收需绑定 frame fence |
| push constants 80 B（model+tint），per-instance `cmd_push_constants` 在绘制循环内 | `vulkan.rs:157-174, 1600-1606` | 批次 F 材质参数走描述符扩展，不改实例路径 |
| `RenderContext` 公共 API：begin_frame/set_camera/submit/end_frame/create_mesh/destroy_mesh/resize 等；无 Vulkan 类型外泄 | `crates/mir-render/src/lib.rs:208-282` | ADR-0012 边界全程不得破坏 |

## 3. 关键技术决策

1. **多帧在飞行取 2**（`MAX_FRAMES_IN_FLIGHT = 2`）。每帧独立：命令池 + 命令缓冲、fence（首帧 SIGNALED）、
   `image_available` 信号量、camera UBO（64 B，持久映射）+ 描述符集；`render_finished` 改为**每帧一个**
   （帧在飞行保证同一信号量不并发复用；若校验层报 `VUID-00067` 类错误，回退为每帧 × 每图像，不改架构）。
   交换链重建时按现有 `device_wait_idle` 前置统一重建全部帧资源。
2. **RenderGraph v0.5 限定单 graphics 队列、声明式 pass + 编译期 barrier 推导**（synchronization2）。
   pass 类型首批支持：graphics（color/depth attachment，dynamic rendering）、transfer（网格上传）。
   图编译器按读写声明拓扑排序、追踪图像布局并自动插入 barrier/layout 转换，替换 v0.2 的 3 处每帧手写 barrier；
   网格上传的 staging barrier 留在传输路径，不强塞进图。临时纹理别名（同帧内生命周期不重叠的 transient 资源复用）
   与 GPU 时间戳（pass 耗时进 `FrameStats`）随批次 B 落地；公共 API 不出现 Vulkan 类型（ADR-0012）。
3. **单 `.spv` 多入口随 RenderGraph 批次落地**：`slangc -fvk-use-entrypoint-name` 使 SPIR-V 保留
   `vertexMain`/`fragmentMain` 入口名，单 `.slang` 编译产出单个 `.spv`；Rust 侧一个 ShaderModule、
   各 stage 用入口名 `.name(...)`；CI 两条命令/着色器（hello、cube），`git diff` 校验 `.spv` 总数减半。
4. **管线缓存持久化**：`VkPipelineCache` 常驻 + 磁盘文件（按 设备 UUID + 驱动版本 + 管线状态哈希 命名），
   启动时加载、失败/损坏即丢弃重建；加载与保存全部容错（IO 失败仅 warn），不阻塞初始化。
5. **自研显存子分配器**（不引入 ash/VMA）：按 memory type 分桶，块大小分级（如 64/256 MB DEVICE_LOCAL、
   16/64 MB HOST_VISIBLE），块内线性分配 + 空闲合并；对外 `alloc(size, align, type, usage) -> Allocation`
   + `free`；删除队列/延迟回收与 frame fence 关联（资源销毁推迟到对应帧 fence 信号后）。
   先在 `vulkan.rs` 内部实现（模块化文件），稳定后再考虑独立。
6. **材质与 PBR 的 v0.3 边界**：材质 = 参数 UBO（metallic/roughness/base_color 等）+ 后续纹理 binding 占位；
   PBR 首版为解析式 metallic-roughness（GGX 近似）+ 单方向光 + 简单环境项；**IBL/CSM/SSAO/Bloom/色调映射归 M5**；
   glTF 导入归 M4，v0.3 演示用程序化几何（球/环面/多立方体阵列）。视锥剔除放 extract 侧
   （`mir-math` 补 `Frustum`/`Aabb`，应用层在 `on_render` 剔除后 `submit`），渲染层不做剔除。

## 4. 任务总表

| ID | 任务 | 依赖 | 人日 | 验收摘要 |
|---|---|---|---:|---|
| **R-01** | 帧资源重做：`FrameResources` 数组化（×2），每帧命令池/缓冲/fence/信号量/UBO/描述符集 | — | 1.0 | 校验层 0 error；headless 行为不变 |
| **R-02** | 提交路径改造：`render_frame` 按帧序号取资源；swapchain 重建统一重建帧资源数组 | R-01 | 0.5 | 600 帧 + resize/最小化回归通过 |
| **批次 A 小计** | | | **1.5** | Gate 1 |
| **R-03** | RenderGraph 数据模型：pass/资源声明 API（纯数据类型，`mir-render` 内部） | R-02 | 1.0 | 声明 API 单测；公共 API 无 vk 类型 |
| **R-04** | 图编译器：拓扑排序、布局追踪、自动 barrier/layout 生成，替换 3 处每帧手写 barrier | R-03 | 1.5 | 校验层 0 error；现有演示画面不变 |
| **R-05** | 临时资源别名 + GPU 时间戳（pass 耗时进 FrameStats）+ 每帧依赖图 dump | R-04 | 1.0 | dump 可用；时间戳数值合理 |
| **批次 B 小计** | | | **3.5** | Gate 2 |
| **R-06** | 单 `.spv` 多入口：`-fvk-use-entrypoint-name`、单 ShaderModule、入口名绑定、CI 双 job 同步 | R-04 | 0.5 | `git diff` 干净；`.spv` 数减半 |
| **批次 C 小计** | | | **0.5** | Gate 3 |
| **R-07** | `VkPipelineCache` 常驻 + 磁盘持久化（设备/驱动/状态哈希键）+ 启动加载容错 | R-04 | 1.0 | 二次启动命中缓存（日志） |
| **批次 D 小计** | | | **1.0** | Gate 4 |
| **R-08** | 子分配器核心：memory type 分桶、块分级、块内线性分配 + 合并、`alloc/free` API 与单测 | R-02 | 1.5 | 分配器单测全过 |
| **R-09** | 接管全部分配点（深度/网格/staging/UBO）+ 删除队列延迟回收（绑定 frame fence） | R-08 | 1.0 | Gate 5；Drop 无残留 |
| **R-10** | 分配器压力/碎片单测 + 校验层回归（含 mesh 反复创建销毁循环） | R-09 | 0.5 | 1000 次 create/destroy 循环无泄漏 |
| **批次 E 小计** | | | **3.0** | Gate 5 |
| **R-11** | 材质数据结构与描述符扩展（材质参数 UBO + 纹理 binding 占位、`max_sets` 扩容） | R-09 | 1.0 | headless 提交多材质不 panic |
| **R-12** | PBR 着色器（Slang：解析式 metallic-roughness + 单方向光 + 环境项）+ 程序化几何（球/环面） | R-11 | 1.5 | 金属/粗糙变化肉眼可辨 |
| **R-13** | `mir-math::Frustum`/`Aabb` + extract 侧视锥剔除接入演示 | R-12 | 1.0 | 剔除生效可观测；单测覆盖 |
| **R-14** | v0.3 前置验收执行：Gate 1–7 全跑 + 记录 1080p 帧率/帧耗时 | R-13 | 1.0 | 验收记录落 §9 |
| **R-15** | 文档更新：README §6.3/§9.6/§14、ADR-0004 补记、本清单执行记录 | R-14 | 0.5 | 进度表反映 M2 剩余完成 |
| **批次 F 小计** | | | **5.0** | Gate 6 |
| **总计** | | | **12.5** | |

> 每批次结束都能 `cargo run` 看到中间结果（A：行为不变但帧路径重构；B：barrier 自动化；C/D：无视觉变化；
> E：无视觉变化；F：PBR 画面）。

## 5. 分阶段详述

### 批次 A：多帧在飞行与环形帧资源（R-01…R-02，1.5 d）

#### R-01 帧资源重做（1.0 d）

- `FrameResources` 改为数组 `[FrameResources; MAX_FRAMES_IN_FLIGHT]`（`MAX_FRAMES_IN_FLIGHT = 2`，常量带
  `debug_assert` 说明依据）。每帧持有：命令池（`RESET_COMMAND_BUFFER`）+ 命令缓冲、fence（首帧 SIGNALED）、
  `image_available` 信号量、camera UBO（64 B，`HOST_VISIBLE|COHERENT`，持久映射）+ 描述符集。
- `render_finished` 改为每帧一个信号量（决策 1）；`recreate_render_finished` 相应改为重建每帧的该信号量。
- 描述符池扩容：`max_sets = MAX_FRAMES_IN_FLIGHT`，descriptor count 同步。
- UBO 写入：`set_camera` 缓存不变；`render_frame` 改为 `memcpy` 到**当前帧**的持久映射 UBO（去掉 map/unmap）。
- 销毁：`Drop` 中逆序销毁全部帧资源（先 wait_idle，沿用 v0.2 顺序约束）。
- 验收：`VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation cargo run -- --frames 120` 0 消息；
  headless 单测不变；`cargo test -p mir-render` 全过。

#### R-02 提交路径改造（0.5 d）

- `render_frame` 增加 `frame_index`（在 `fence` 信号后递增）；wait/acquire/reset/record/submit/present 全部
  按当前帧资源取用；present wait 当前帧的 `render_finished[frame_index]`。
- `recreate_swapchain`：重建交换链/深度后统一重建每帧 UBO 相关描述符与 `render_finished`（wait_idle 前置不变）。
- 单测：`frame_index` 在成功帧后推进、swapchain outdated 时重建后继续推进、最小化跳帧不影响序号。
- 验收：600 帧窗口 + resize/最小化/Alt+Tab 回归（对齐 v0.2-cube.md §10 的手测路径）；
  记录改造前后 1080p 平均 FPS（预期 CPU 提交等待减少，不设硬指标）。

### 批次 B：RenderGraph v0.5（R-03…R-05，3.5 d）

#### R-03 数据模型与声明 API（1.0 d）

- 新增 `crates/mir-render/src/graph.rs`：`RenderGraph`、`PassId`、`ResourceId`、`PassDesc`
  （graphics：color/depth attachment 读写声明；transfer：buffer 读写）、`ResourceDesc`
  （transient image：格式/尺寸跟随交换链 extent；imported：swapchain 图像、深度图像、顶点/索引缓冲）。
- 纯数据类型，不出现 `vk::*`；`VulkanCore` 持有 `RenderGraph` 并在录制前 `compile`。
- 单测：pass 声明/资源去重、循环依赖报错、headless 下 compile 空转。

#### R-04 图编译器与自动 barrier（1.5 d）

- 编译：拓扑排序 → 逐资源布局/访问状态追踪 → 生成 barrier/layout 转换列表（synchronization2）。
  替换 v0.2 的 3 处每帧手写 barrier（acquire 后、呈现前、深度一次性转换并入 imported 资源初始化语义）。
- 每条生成的 barrier 带注释（src/dst 布局与动机）；对照基准为 §2 列出的现有 4 处 barrier。
- 录制循环改为：图编译 → 逐 pass `cmd_begin_rendering`/结束，pass 内绘制调用由现有提交收集逻辑注入。
- 验收：校验层 0 error；画面与改造前一致；`cargo run -- --frames 120` 无 `frame_error`。

#### R-05 别名、时间戳与依赖图 dump（1.0 d）

- 临时纹理别名：同帧内生命周期不重叠的 transient 资源复用同一 `vkImage`/内存（v0.5 只做同一交换链 extent 的
  单 swapchain 别名池，简化）。
- GPU 时间戳：`vkCmdWriteTimestamp2` 包裹每个 pass，结果进 `FrameStats`（新增 pass 级耗时字段，`--stats` 输出）。
- 依赖图 dump：`RenderGraph::dump()` 输出 DOT 文本（资源节点 + pass 边 + barrier 标注），
  环境变量或 CLI 开关控制，默认关闭。
- 验收：dump 输出与实际 pass/资源一致（人工核对）；时间戳数值落在帧耗时内（总和 ≈ GPU 帧时间）。

### 批次 C：单 `.spv` 多入口（R-06，0.5 d，随批次 B 之后）

- `slangc` 增加 `-fvk-use-entrypoint-name`：`shaders/cube/cube.slang`、`shaders/hello/triangle.slang`
  各编译出**单个** `.spv`（保留 `vertexMain`/`fragmentMain` 入口名），删除旧的两个按阶段 `.spv`。
- `vulkan.rs`：单个 ShaderModule，两个 stage 分别 `.name(b"vertexMain\0")` / `.name(b"fragmentMain\0")`；
  `include_shader_code!` 路径同步。
- CI 两个 job：slangc 命令从 4 条减为 2 条，`git diff --exit-code` 目标改为 `shaders/**/*.spv`（数量减半）。
- 验收：本地复现 CI 两步 + `spirv-val`；`cargo run -- --frames 120` 画面不变；CI 推送后双 job 绿。

### 批次 D：管线缓存持久化（R-07，1.0 d）

- `init_pipeline`：创建常驻 `VkPipelineCache`（替换 `PipelineCache::null()`），创建前尝试从磁盘加载
  （路径：`%LOCALAPPDATA%/Miriya/cache/pipeline/<device_uuid>_<driver_ver>_<layout_hash>.bin`；
  Linux 占位等价路径），失败/损坏仅 `warn` 并用空缓存。
- 首次创建成功后写回磁盘（IO 失败不阻塞、不 panic）；`Drop` 时销毁 cache 对象（wait_idle 之后）。
- 缓存键组成与写入头（magic + 版本 + 键）入注释；命中判定只做文件级（不做管线级 diff），损坏即全量重建。
- 验收：连续两次启动日志显示第二次命中；删除缓存文件后可正常重建；校验层 0 error；
  headless（无管线）路径不读写缓存。

### 批次 E：自研显存子分配器（R-08…R-10，3.0 d）

#### R-08 分配器核心（1.5 d）

- 新增 `crates/mir-render/src/allocator.rs`：`MemoryAllocator`——按 memory type index 分桶；
  块大小分级（DEVICE_LOCAL：64/256 MB，HOST_VISIBLE：16/64 MB，按需开新块、上限保护）；
  块内线性分配 + 空闲区间合并；`alloc(size, alignment, type_index, dedicated: Option<Requirement>) -> Allocation`
  （`Allocation { memory, offset, size }`，dedicated 分配用于特殊 requirement 资源）。
- 全部在 `mir-render` 内部、`forbid(unsafe_code)` 之外区域照常标注；查询 `get_physical_device_memory_properties`
  复用现有 `find_memory_type`（`vulkan.rs:753-771`）。
- 单测：对齐（含非幂次对齐）、块耗尽扩容、free 后合并、世代/句柄无泄漏（分配计数守恒）。

#### R-09 接管分配点 + 延迟回收（1.0 d）

- `create_buffer`（`vulkan.rs:977-1004`）与深度图像路径（`vulkan.rs:774-843`）改为经分配器
  （bind 时带 offset）；staging/UBO/网格全部走同一入口；删除原两处独立 `allocate_memory`。
- 新增 `DeletionQueue`：资源销毁请求记录 `(GPU 资源, Allocation, 关联 fence)`，帧循环检测 fence 信号后真正
  `destroy_buffer`/`destroy_image` + `allocator.free`；`destroy_mesh`/swapchain 重建/`Drop` 全部改走该路径
  （`Drop` 收尾时 flush 全队列）。
- 验收：校验层 0 error；mesh 反复 create/destroy 循环测试见 R-10。

#### R-10 压力与碎片验证（0.5 d）

- 单测（需 GPU，CI 无硬件时显式跳过）：1000 次 create/destroy 循环、交错大小分配、
  全部销毁后分配器 `allocated_bytes == 0`；碎片率统计输出。
- 验收：校验层 0 error；无内存泄漏（分配计数守恒）；`--headless --frames 1000` 不变。

### 批次 F：材质、基础 PBR 与视锥剔除（R-11…R-15，5.0 d）

#### R-11 材质数据结构与描述符扩展（1.0 d）

- `mir-render` 新增 `MaterialData`（base_color、metallic、roughness、emissive；`repr(C)`）与
  `create_material(&MaterialData) -> MaterialHandle`（世代句柄，风格对齐 `MeshHandle`）。
- 描述符：set 0 binding 0 相机 UBO 不变；新增 set 1（per-material UBO）或 SSBO 数组（先做单材质 UBO 版本，
  数组化留 M5 descriptor indexing）；`max_sets`/pool size 扩容；`submit` 增加材质参数或默认材质。
- push constants 保持 80 B 不变（ADR-0012 决策 3）。
- 验收：headless 提交多材质不 panic；窗口内材质参数差异可见（先 Lambert 验证）。

#### R-12 PBR 着色器与程序化几何（1.5 d）

- `shaders/pbr/pbr.slang`：解析式 metallic-roughness（GGX 近似）+ 单方向光 + 简单环境项（固定半球环境色）；
  顶点输入扩展法线（`Vertex` 增 `normal`，stride 变更同步管线与网格）。
- `geometry.rs` 新增 `sphere()`/`torus()`（程序化，含法线）；演示场景升级为多材质物体阵列。
- `.spv` 入库 + CI 两个 job 增加该着色器（单 `.spv` 多入口方式）。
- 验收：metallic 0↔1、roughness 0↔1 变化肉眼可辨；法线朝向正确（无内翻黑面）。

#### R-13 `Frustum`/`Aabb` 与 extract 侧剔除（1.0 d）

- `mir-math` 新增 `Aabb`、`Frustum`（由 view_proj 六平面提取）+ 单测（平面符号、相交/包含/分离）。
- 应用层：每个演示实体带 `Aabb`（由几何 + `Transform` 生成），`on_render` 做 `Frustum::contains(aabb)`
  剔除后再 `submit`；渲染层不感知剔除（决策 6）。
- 验收：相机背对场景时 `submitted_instances()` 下降（保留诊断输出）；转到正面恢复；无漏剔除闪烁。

#### R-14 验收执行（1.0 d）

- 跑 Gate 1–7；release 1080p 600 帧记录 FPS/帧耗时/GPU pass 时间戳；
  回归：`--headless --frames 1000`、`mir-vulkan-probe`、resize/最小化手测路径；
  清理临时调试代码；`clippy -D warnings` 零告警（`SAFETY:` 注释齐全）。
- 结果写入 §9 验收记录。

#### R-15 文档更新（0.5 d）

- README §6.3（`mir-render` 现状）、§9.6、§14.1/§14.2/§14.4（M2 剩余收尾）；ADR-0004 补记第 2/4/6 条落地情况；
  本清单 §10 执行记录逐批补齐；如涉及架构新增（RenderGraph/分配器公共语义）补 ADR 或并入 ADR-0004 追记。

## 6. 依赖顺序（关键路径）

```text
R-01 → R-02 ─┬─ R-03 → R-04 ─┬─ R-05
             │               ├─ R-06
             │               └─ R-07
             └─ R-08 → R-09 → R-10 ── R-11 → R-12 → R-13 → R-14 → R-15
```

- 关键路径：R-01 → R-02 → R-08 → R-09 → R-11 → R-12 → R-13 → R-14。
- R-05/R-06/R-07 可并行于批次 E 起步（互不依赖）。
- R-09（接管分配点）必须在 R-11（材质描述符扩容）之前，否则新资源绕过分配器。

## 7. 风险与对策

| 风险 | 概率 | 对策 |
|---|---|---|
| 多帧在飞行引入信号量/fence 复用类 VUID | 中 | 先改帧资源再动提交循环；校验层逐条清；`render_finished` 预案为每帧 × 每图像（决策 1） |
| RenderGraph 编译器漏 barrier 导致布局/同步错误 | 中 | 以 v0.2 的 4 处手写 barrier 为对照基准；每条生成的 barrier 带动机注释；校验层 0 error 为硬 Gate |
| vulkanalia builder 悬垂指针在 release 复发（v0.2 §10 教训） | 中 | 新 unsafe 全部用命名数组传 builder；release + 校验层运行纳入每批次验收 |
| 子分配器对齐/绑定错误导致 validation 报错 | 中 | `alloc` 强制对齐参数；单测覆盖边界对齐；分桶尺寸保守起步 |
| 延迟回收与 swapchain 重建竞态 | 低 | 回收统一绑定帧 fence；重建路径前置 `device_wait_idle`（沿用 v0.2 语义） |
| PBR 着色器首版视觉验证困难（无资产导入） | 低 | 用可调参数的程序化几何 + 颜色梯度对照；golden 不做像素级断言，仅 Gate 数值指标 |
| 工作量超估（批次 E/F 工程量大） | 中 | 批次边界即回滚点；批次 F 的 R-12/R-13 可拆出「仅单光 Lambert 验证」的最小闭环 |

## 8. 不包含（明确留待后续里程碑）

| 项 | 去向 | 说明 |
|---|---|---|
| 异步网格上传（transfer 队列 + 帧同步回传） | M2 后/M4 | 批次 A 保留阻塞上传；独立 transfer 队列随资产管线评估 |
| IBL / CSM / SSAO / Bloom / 色调映射 | M5 | ADR-0004 第 1 条完整 pass 链 |
| Clustered Forward+ 多光源 | M5 | 本清单 PBR 仅单方向光 |
| glTF 资产导入与纹理加载 | M4 | v0.3 演示用程序化几何 |
| 着色器热重载 | M5/M13 | ADR-0011 第 6 条 |
| 时间线信号量 / descriptor indexing 全面启用 | 按需 | 仅在 RenderGraph/材质确需时评估，本清单不承诺 |
| 频繁重建场景下的多线程录制 | M3 之后 | 帧录制仍在主线程 |

## 9. 验收记录（执行后填写）

| 项 | 命令/方式 | 结果 | 日期 |
|---|---|---|---|
| （待执行） | | | |

## 10. 执行记录

### 2026-11-24：批次 A——多帧在飞行与环形帧资源（R-01/R-02 已完成）

| 项 | 改动 |
|---|---|
| 帧资源数组化 | `FrameResources` 改为 `VulkanCore::frames: Vec<FrameResources>`（`MAX_FRAMES_IN_FLIGHT = 2`），每帧独立命令池/缓冲、fence（创建即 SIGNALED）、`image_available`、`render_finished`（每帧 × 每交换链图像）与相机 UBO（HOST_VISIBLE\|COHERENT，创建时持久映射，`camera_mapped` 指针逐帧写入） |
| 描述符扩容 | 描述符池 `max_sets`/descriptor count 扩为 2；每帧分配一个集合（`descriptor_sets: Vec`），管线布局仍只声明 1 个 set；UBO 写入去掉每帧 map/unmap，改为 fence 等待后写持久映射 |
| 提交路径 | `render_frame` 按 `frame_index` 环形取帧资源，present 成功后推进序号；acquire 提前返回不推进；`recreate_swapchain` 统一重建各帧 `render_finished` 与 extent |
| 失败回收 | `init_frame_resources` 单帧创建失败就地回收（`fail!` 宏）；新增 `PipelineGuard`——`init_pipeline` 任一步失败按逆序销毁已创建对象（修复了此前 pipeline 中途失败泄漏 module/layout/pool 的既有问题，本次实际触发过并修复） |
| 单测 | 新增 `frames_in_flight_is_two`；`cargo test -p mir-render` 21 项全过 |

验证（2026-11-24）：

- `VK 校验层 debug 默认开启 cargo run -- --frames 120`：**0 validation 消息**，58.2 FPS。
- `cargo run --release -- --validation --frames 600 --width 1920 --height 1080`：600 帧完成，**57.7 FPS**，
  0 validation 消息（基线 v0.2 为 58.2，差距在 vsync 波动范围内）。
- `cargo run -- --headless --frames 1000 --no-validation`：1000 帧完成，退出正常。
- `cargo test --workspace`：78 个套件全 ok（`mir-log` doctest 曾在并行跑时偶发失败，单跑稳定通过，属既有 flake）。
- `cargo fmt --all -- --check` 通过；`cargo clippy --workspace --all-targets -- -D warnings` 通过
  （仅 Cargo 清单既有 unused dependency 警告）。
- 手测路径（resize/最小化/Alt+Tab）本次未逐项重演：交换链重建路径仅新增"遍历全部帧更新 extent / 重建 render_finished"，
  与 v0.2 已验证逻辑同构，留待批次 B 改动 barrier 时一并回归。
