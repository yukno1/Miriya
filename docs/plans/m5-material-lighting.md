# M5 任务清单（材质与光照：纹理 → 材质绑定/多光源 → RenderGraph 多 pass → Bloom/SSAO → IBL → CSM → v0.3 PBR Scene）

- 日期：2026-11-27（批次 0：计划与复核同日完成）
- 状态：**批次 A 完成（R-01–R-03 落地，Gate 1 通过）——待批次 B 启动**（2026-11-27；2026-11-28 同步双后端前提）
- 上游依据：`README.md` §8.2（M5）/ §8.3（v0.3 PBR Scene）/ §9.6 / §14.2；
  `docs/plans/m2-remaining.md`（RenderGraph/子分配器/材质雏形交接）、`docs/plans/m4-asset-pipeline.md` §5（M5 交接清单）；
  ADR-0004（RenderGraph）、ADR-0011（Slang）、ADR-0012（帧数据/提交边界）、ADR-0013（资产管线）、
  ADR-0014（wgpu 首选双后端，取代本计划写作时的单后端约束）。
- 约束：Windows x64 + **wgpu 首选 / Vulkan 1.3（vulkanalia）回退**、Slang 单 `.spv` 多入口、单 graphics 队列；
  `mir-render` 公共 API 不出现任何图形 API 类型（ADR-0012）；Vulkan 侧显存走自研子分配器 + DeletionQueue，
  wgpu 侧由 crate 跟踪资源生命周期；不引入 ash/VMA/第三套原生图形后端；资产层只产出纯数据，
  GPU 创建由渲染层完成。
- **批次 B 起的双后端要求**：本计划 R-04 之后的每项渲染能力都需在默认 wgpu 路径可用，Vulkan 侧保持等价；
  涉及新绑定/管线状态时同时更新 `wgpu/pipeline.rs` 与 `vulkan/pipeline.rs`。细节以 ADR-0014 为准。
- 估算：**约 24 净人日**（批次 A 3.0 + B 3.0 + C 4.0 + D 3.0 + E 4.0 + F 3.5 + G 2.5，含 1.0 冗余）

## 1. 目标与 Gate

**v0.3 目标**：把 M4 交接的渲染缺口全部落地——纹理 GPU 上传/采样、材质纹理绑定、多光源、IBL、CSM、SSAO、
Bloom/色调映射、Slang module 化——并接入 glTF 资产驱动的演示场景，达成 v0.3「PBR Scene」垂直切片：
**glTF 导入 + IBL + CSM + 色调映射，1080p 60 FPS（中端独显）**。

**Gate（全部满足才算 M5 达成）**

1. 纹理上传与采样落地：`create_texture`（RGBA8 / KTX2 带 mip 链）→ 采样绑定 → 贴图肉眼可见；
   staging/图像显存走子分配器，销毁经 DeletionQueue 绑定 frame fence，校验层 0 error。
2. 材质纹理绑定 + 多光源：无纹理材质回退默认白纹理；≥ 2 光源场景光斑/方向变化肉眼可辨；
   光照数据走新 set 2 UBO，`RenderContext` 新增光照门面 API，公共 API 无 Vulkan 类型。
3. RenderGraph 多 pass：`Usage`/`PassKind`/`ImageFormat` 扩展后，每帧链路
   `geometry(HDR) → SSAO → bloom → tonemap → 交换链` 全部由图编译器生成 barrier；
   `--graph-dump` DOT 能反映多 pass 与 transient 别名；每个新增 pass 耗时进 `FrameStats.gpu_pass_timings`。
4. 效果可辨：SSAO 接触阴影、Bloom 高光泛光、ACES 色调映射、IBL 金属/粗糙反射变化、CSM 阴影边缘
   （3×3 PCF 无 banding）均可与关闭状态对比区分。
5. glTF 资产链路：`.miasset` 驱动的 GLB 场景（网格 + 材质参数 + 纹理）经 M4 异步加载接入演示，
   画面稳定且 free-camera/校验层不退化。
6. 性能：`cargo run --release -- --validation --frames 600 --width 1920 --height 1080 --stats`，
   实测 FPS 记录并对比 M4 基线 58.3 FPS（允许 ≤ 3 FPS 波动换取视觉效果；不满足时按 §6 降级裁剪）。
7. 全程回归：每批次结束跑 `cargo fmt --all -- --check`、
   `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全绿；
   `--headless --frames 1000 --no-validation` 行为不变（新纹理 API headless 空操作 + 计数器语义）；
   新增 `unsafe` 均带 `SAFETY:` 注释；CI 两个 job 的 `slangc` 命令与 `git diff --exit-code -- shaders/**/*.spv` 同步更新。

## 2. 现状核对（2026-11-27 code-explorer 已读代码确认的事实）

| # | 事实 | 位置 | 对 M5 的影响 |
|---|---|---|---|
| 1 | `MaterialData` 为 48 B 纯 f32 std140（base_color/metallic/roughness/emissive），偏移有单测锁定；无纹理字段；注释明确纹理绑定「留待 M5 descriptor indexing 数组化」 | `mir-render/src/material.rs:3-5,14-33,95-110` | 批次 B 扩展结构 + 单测更新 |
| 2 | `MaterialHandle(u64)` = 低 32 位槽位 + 高 32 位世代；`destroy_material` 世代递增、buffer 走 DeletionQueue、描述符集 fence 信号后归还池 | `material.rs:59-76`、`vulkan/material.rs:163-191` | 纹理句柄照此模板实现 |
| 3 | set 1 布局仅 binding0 = UNIFORM_BUFFER（FRAGMENT）；描述符池仅 UNIFORM_BUFFER 一个 pool_size（`MAX_FRAMES_IN_FLIGHT + MAX_MATERIALS`），**无 COMBINED_IMAGE_SAMPLER 容量**；管线布局 2 sets + 80 B push constant | `vulkan/pipeline.rs:232-278,328-341` | 批次 B 扩布局 + 扩池；`MAX_MATERIALS = 64`（`vulkan/material.rs:16`） |
| 4 | 绘制循环按 draw 解析材质句柄 → set 1 按需重绑（`bound_material_set` 缓存）；逐实例 model/tint 走 push constants | `vulkan/render.rs:257-289` | 批次 B/C 绑定缓存策略沿用 |
| 5 | 全仓库**无** color/sampled image 创建、**无** `create_sampler`；唯一图像创建路径是深度图像（2D、单 mip、DEPTH_STENCIL_ATTACHMENT usage + ImageView） | `vulkan/depth.rs:69-86,131-145` | 批次 A 以 depth.rs 为模板建通用纹理路径 |
| 6 | 图像布局转换只有 `transition_depth_layout`（一次性传输命令缓冲 + `queue_wait_idle`） | `vulkan/depth.rs:167-218` | 批次 A 模板化为通用转换；mip blit 后布局转换复用 |
| 7 | 分配器 `alloc(size, align, type_index, dedicated)` 与资源类型无关，`max(alignment, bufferImageGranularity)` 已内建；块分级 DEVICE_LOCAL 64/256 MiB、HOST_VISIBLE 16/64 MiB | `vulkan/allocator.rs:7-9,21-23,212-218` | 批次 A 直接支持 IMAGE 分配，无需改分配器核心 |
| 8 | `ImageData { width, height, rgba }` 仅 RGBA8 无 mip/格式；`Ktx2Data { width, height, vk_format, bytes }` bytes 为**整文件透传**（无 mip 层数/块尺寸解析） | `mir-image/src/lib.rs:27-54`、`mir-image/src/ktx2.rs:21-49` | 批次 A 需补 KTX2 mip/格式元数据解析 |
| 9 | `NativeAsset` 已含 `Image/Ktx2` 变体且导入链已通；`MaterialAsset` 直接复用 `MaterialData` | `mir-asset/src/asset.rs:20-26,61-66`、`importer/mod.rs:48-64` | 纹理字段占位只需动 `MaterialData` 侧 |
| 10 | `PassKind` 仅 Graphics{color,depth}/Transfer；`Usage` 仅 4 种（ColorWrite/DepthWrite/BufferRead/BufferWrite），`stage_access()` 映射无 fragment/compute 阶段；`ImageFormat` 仅 R8G8B8A8Unorm/D32Sfloat | `mir-render/src/graph.rs:37-41,183-260` | 批次 C 扩枚举与映射 |
| 11 | 图编译器：拓扑排序 + RAW/WAR/WAW 边 + 布局追踪生成 `pre_pass_barriers`/`final_barriers`；transient 图像别名（贪心槽位复用）已实现但**从未实际使用**；`dump_dot` 完备 | `graph.rs:445-680,640-672,683-777` | 批次 C 首次接入 transient HDR 目标 |
| 12 | 每帧 `build_frame_graph` 仅 1 个 `main_geometry` graphics pass；barrier 翻译 `map_stages/map_access` 需补 fragment/compute；GPU 时间戳每 pass 2 个查询回读进 `pass_timings()` | `vulkan/frame_graph.rs:20-64,87-130,132-174`、`render.rs:103-160`、`lib.rs:229-235`；`TIMESTAMP_QUERY_COUNT = 64`（`vulkan/frame.rs:13`） | 批次 C 扩展；注意 QueryPool 容量按 pass 数动态化 |
| 13 | pbr.slang：绑定 set0 相机 UBO 80B、set1 材质 UBO 48B、push constant 80B；光照为**着色器内常量** `LIGHT_DIRECTION/LIGHT_COLOR`；环境项固定半球插值；无 include/module 机制，纯单文件 | `shaders/pbr/pbr.slang:1-11,34-63,103-132` | 批次 B 光照改 UBO；批次 C/E 引入 Slang module |
| 14 | CI 两 job 各 `slangc -profile glsl_450 -target spirv -fvk-use-entrypoint-name` + `git diff --exit-code`；`.spv` 经 `include_shader_code!` 编译期嵌入 | `.github/workflows/ci.yml:30-36,90-93`、`vulkan/pipeline.rs:116-154` | 新增着色器须同步两 job |
| 15 | 应用层 `on_render`：惰性 mesh/材质 → `level.world_transforms()` → begin_frame → set_camera → 视锥剔除 → InstanceData → submit_with_material → end_frame；`--stats/--headless/--graph-dump` CLI 已就绪 | `miriya-app/src/main.rs:279-394,417-452` | 批次 G glTF 场景接入点 |
| 16 | 帧循环 `tick()`：`assets.poll()` → on_update → on_render → stats.update；`FrameStats.gpu_pass_timings: Vec<(String, f32)>` | `mir-core/src/lib.rs:118-140`、`mir-core/src/time.rs:51-59` | 无需改动，pass 耗时自动进统计 |

## 3. 关键技术决策

1. **纹理数据入口走 `mir-render` 自有纯数据类型**（ADR-0012）：新增 `TextureData { format: ImageFormat,
   width, height, mip_data: Vec<(offset,len)>, bytes: Vec<u8> }`；应用层从 `mir-image` 的 `ImageData`（RGBA8）
   与增强后的 `Ktx2Data`（补 mip 层数/层数/块信息解析）转换，`mir-render` 不依赖 `mir-image`。
   `ImageFormat` 枚举扩充：R8G8B8A8Unorm / B10G11R11Ufloat / R16G16B16A16Sfloat / R32Sfloat / D32Sfloat。
2. **纹理句柄照抄 mesh/material 模板**：`TextureHandle(u64)` 槽位+世代；`create_texture(TextureData) ->
   Result<TextureHandle, RenderError>` 内部：staging（HOST_VISIBLE，经分配器）→ 一次性传输命令缓冲
   `copy_buffer_to_image`（逐 mip 或 blit 生成 mip 链，首次上传沿用阻塞路径，与 mesh 上传一致；
   异步上传不在 M5）→ `QUEUE_WAIT_IDLE` 后 staging 经 DeletionQueue 延迟回收；
   image + view + sampler（LINEAR repeat）常驻。`destroy_texture` 经 DeletionQueue 绑定 frame fence。
3. **材质绑定不用 descriptor indexing**（v0.3 不需要运行时无界数组）：set 1 扩为
   binding0 UBO（48 B 不变）+ binding1..3 CombinedImageSampler（albedo / normal / metallic-roughness），
   不足处绑定默认纹理（1×1 白 / 1×1 平面法线 / 1×1 白）；描述符池补 COMBINED_IMAGE_SAMPLER
   `3 × MAX_MATERIALS`、max_sets 相应扩大；`MAX_TEXTURES = 128` 上限（纹理槽位表）。
   材质 UBO 加 `base_color` 已有，无需为无纹理路径改布局；`MaterialData` 加
   `albedo/normal/metallic_roughness: TextureHandle`（u64 字段追加，std140 不受影响，偏移单测更新）。
4. **多光源走新 set 2**：`LightsUbo`（std140：`u32 count` + 8 × {type:u32, position:vec3,
   direction:vec3, color:vec3, intensity:f32, params:vec4}，估算 ≤ 640 B）每帧一份（×MAX_FRAMES_IN_FLIGHT，
   持久映射）；管线布局扩为 3 sets；`RenderContext::set_lights(&[LightData])` 新门面 API。
   pbr.slang 光照循环遍历方向光/点光（Lambert + Blinn-Phong 高光衰减并入 GGX 路径）。
5. **RenderGraph 扩采样语义**：`Usage` 增 `ShaderRead`（fragment sampled，映射
   FRAGMENT_SHADER/SHADER_READ/SAMPLED_IMAGE_OPTIMAL）与 `ShaderWrite` 预留；`PassKind::Graphics`
   增 `sampled_reads: Vec<ResourceId>`；`Stages` 增 FRAGMENT_SHADER，`frame_graph.rs` 的
   `map_stages/map_access/layout` 补映射。后处理统一用**全屏三角形 fragment pass**（单 graphics 队列，
   不引入 compute pass，理由：现有管线/动态渲染/时间戳路径零新增）。
6. **HDR 管线 + 色调映射**：geometry pass 改渲染到 transient `R16G16B16A16Sfloat` HDR 目标
   （首次实际使用 transient 别名机制）→ tonemap pass（ACES + gamma）输出到 imported 交换链图像。
   Bloom：HDR 目标下采样 mip 链（每 mip 一个 transient pass）→ 上采样叠加（additive blend），
   mip 数按分辨率对数（1080p 约 5–6 级）；SSAO：半分辨率 transient（R8Unorm）+ depth 输入 +
   16 样本旋转随机 + 双边模糊 pass。所有 pass 每帧重建进 `build_frame_graph`，耗时自动进 `FrameStats`。
7. **IBL 预计算**：equirect HDR（用 `B10G11R11Ufloat` 或 RGB16F 纹理承载，源自 PNG/HDR 导入的
   `TextureData`）→ cubemap 6 面 conversion pass（一次性，启动时录到独立命令缓冲并 wait_idle）→
   irradiance cubemap（32²，diffuse 卷积）+ prefiltered specular cubemap（128²，5 mip，
   roughness 分档）+ BRDF LUT（512²，2 次全屏 pass）均一次性预计算、常驻显存、
   经分配器 dedicated 分配；pbr.slang 环境项替换为 `env_diffuse * irradiance + env_specular * prefiltered + LUT`。
8. **CSM 取 4 级联 × 2048²**：光照 UBO 扩展 4 个 shadow 矩阵 + split 距离（按相机远近裁剪指数/线性混合划分）；
   shadow pass 为 depth-only（D32Sfloat transient，每级联一个），几何 pass 内按像素级联选择 + 3×3 PCF；
   shadow pass 与主 pass 同帧入 RenderGraph（depth 输出 → ShaderRead 采样，验证深度纹理采样 barrier 正确性）。
9. **Slang module 化**：新增 `shaders/common/`（PBR 函数、光照、色调映射、全屏三角形顶点入口），
   `slangc` 以 `shaders/<x>/<x>.slang` 引用 `../common/common.slang` 相对 include；
   新增后处理着色器：`ssao.slang`、`bloom_down/up.slang`、`tonemap.slang`、`shadow.slang`、
   `env_convert/irradiance/prefilter/brdf.slang`；CI 两 job 的 slangc 命令逐个同步，`git diff` 清单扩充。
10. **v0.3 演示场景**：`miriya-app` 保留程序化回退，资产就绪时切换为 GLB 场景
    （经 M4 异步加载 + poll），材质走纹理绑定路径；默认无场景文件时沿用现有 PBR 阵列（不退化）。
    效果开关不做运行时 CLI（避免公共 API 膨胀），效果验证以批次内画面对比 + `--stats` pass 耗时为据。

## 4. 任务总表

| ID | 任务 | 依赖 | 人日 | 验收摘要 |
|---|---|---|---:|---|
| **R-01** | `ImageFormat`/`TextureData` 纯数据类型 + KTX2 mip/块信息元数据解析（`mir-image` 增强） | — | 1.0 | 格式映射单测；malformed KTX2 返回可读错误 |
| **R-02** | `create_texture`/`destroy_texture`：image+view+sampler+mip 链上传，子分配器 + DeletionQueue，headless 空操作 | R-01 | 1.5 | 校验层 0 error；纹理槽位世代句柄单测 |
| **R-03** | 上传路径回归：分配器图像分配压力单测（create/destroy 循环） | R-02 | 0.5 | 1000 次循环无泄漏、无残留 |
| **批次 A 小计** | | | **3.0** | Gate 1 |
| **R-04** | set 1 扩展（3×CombinedImageSampler）+ 描述符池扩容 + 默认纹理（白/平面法线） | R-03 | 1.0 | 无纹理材质不 panic；校验层 0 error |
| **R-05** | `MaterialData` 加 3 个纹理句柄字段 + `submit_with_material` 纹理路径 + glTF 材质链路（`mir-asset` 同步） | R-04 | 1.0 | 贴图/法线/金属粗糙贴图肉眼可辨；偏移单测更新 |
| **R-06** | `LightsUbo`（set 2，每帧 UBO）+ `LightData`/`set_lights` 门面 API + pbr.slang 多光源循环 + `shaders/common` module 化 | R-04 | 1.0 | ≥ 2 光源光斑可辨；CI slangc 同步、`git diff` 干净 |
| **批次 B 小计** | | | **3.0** | Gate 2 |
| **R-07** | `Usage::ShaderRead`/`Stages::FRAGMENT_SHADER`/`PassKind::Graphics.sampled_reads`/`ImageFormat` 扩充 + `map_stages/map_access/layout` 补映射 + 图编译器单测 | 批次 B | 1.5 | 编译器单测（含采样读→写环检测）；公共 API 无 vk 类型 |
| **R-08** | 全屏三角形管线 + HDR transient 目标 + geometry→tonemap 两 pass 链 + `build_frame_graph` 扩展 | R-07 | 1.5 | `--graph-dump` 显示 2 pass + transient 别名；画面与直通 tonemap 对比可辨（ACES）；校验层 0 error |
| **R-09** | 多 pass 时间戳容量核对与回读正确性（QueryPool 扩容按 pass 数） | R-08 | 1.0 | 每新 pass 耗时进 `FrameStats` 且数值合理 |
| **批次 C 小计** | | | **4.0** | Gate 3 |
| **R-10** | SSAO：半分辨率 transient + depth 输入 + 16 样本 + 双边模糊 pass | R-09 | 2.0 | 接触阴影可辨；SSAO pass 耗时进 stats |
| **R-11** | Bloom：HDR mip 下采样链 + 上采样叠加（transient 别名复用） | R-09 | 1.0 | 高光泛光可辨；mip pass 数与 alias 槽位在 dump 中正确 |
| **批次 D 小计** | | | **3.0** | Gate 4（部分） |
| **R-12** | equirect→cubemap conversion + irradiance/prefiltered/BRDF LUT 一次性预计算（独立命令缓冲 + wait_idle） | R-09 | 2.0 | 预计算产物常驻、校验层 0 error、二次启动耗时记录 |
| **R-13** | pbr.slang IBL 项接入（替换固定半球环境项）+ 环境贴图资产链路（HDR/equirect 导入 → TextureData） | R-12 | 2.0 | 金属/粗糙在 IBL 下反射变化肉眼可辨 |
| **批次 E 小计** | | | **4.0** | Gate 4（完成） |
| **R-14** | shadow map 4 级联（D32Sfloat transient）+ depth-only shadow pass + 光照 UBO shadow 矩阵扩展 | R-13 | 2.0 | 深度采样 barrier 正确；校验层 0 error |
| **R-15** | 主 pass 级联选择 + 3×3 PCF + split 调参 | R-14 | 1.5 | 阴影边缘无 banding；近远过渡可接受 |
| **批次 F 小计** | | | **3.5** | Gate 4（CSM） |
| **R-16** | v0.3 演示场景：GLB 资产（网格+材质+纹理）异步加载接入 + 程序化回退保留 | R-15 | 1.5 | 资产就绪切换场景；headless/校验层不退化 |
| **R-17** | Gate 1–7 全跑 + 性能记录（1080p 600 帧 FPS、每 pass 耗时）+ README/ADR-0004/0011/0012 同步 | R-16 | 1.0 | §6 验收记录表填满；Gate 全过 |
| **批次 G 小计** | | | **2.5** | Gate 6/7 |
| **总计** | | | **23.0** | |

## 5. 依赖顺序

`计划 → 批次 A（纹理） → 批次 B（材质/多光源） → 批次 C（RenderGraph 多 pass） → 批次 D（SSAO/Bloom） → 批次 E（IBL） → 批次 F（CSM） → 批次 G（场景/验收）`。

- 批次 A/B 为 GPU 资源层（不依赖图扩展），可先行；批次 C 是 D/E/F 的共同前置（采样语义 + HDR + 多 pass）。
- E（IBL）与 D（SSAO/Bloom）依赖 R-09 后可并行（均只动着色器与 `build_frame_graph` 追加 pass）。
- 每批次结束先跑 `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo fmt --all -- --check`，全绿再进下一批；涉及窗口路径的批次（A/B/C/D/E/F）另跑
  `--validation --frames 120`（校验层 0 error）与 `--headless --frames 1000 --no-validation`。

## 6. 风险与对策

| 风险 | 对策 |
|---|---|
| HDR transient + 多 pass 后帧率跌破基线（M4：58.3 FPS） | 性能预算前置：批次 C 后先测 1080p 基线增量；不达标时按「SSAO 半分辨率→1/2 mip 数→Bloom 关断链」顺序裁剪，并记录取舍；禁止为凑帧率砍校验层覆盖 |
| 深度纹理采样（CSM）的 barrier/布局正确性易错 | 批次 C 先用颜色 RT 采样打通 `ShaderRead` 全链路（tonemap 输入），F 才引入 depth sampled；`--graph-dump` 每批人工核对布局链 |
| KTX2 透传数据缺元数据导致 mip/格式解析出错 | R-01 补解析时对不支持 vkFormat 返回 `RenderError::Unsupported`，负例单测先行；先 RGBA8 直通保底 |
| 一次性 IBL 预计算阻塞启动 | 预计算仅启动时一次（非每帧），1080p 级产物估算 < 100 ms；超预算时降级 irradiance 32²→16²、prefilter mip 5→3，并记录耗时 |
| descriptor 池扩容后 `MAX_MATERIALS=64` 不够 | `MAX_TEXTURES=128` 与材质数解耦；材质描述符集仍按材质分配，超限报错语义与现状一致（`create_material` 返回错误，不 panic） |
| 全屏三角形管线与现有动态渲染/视口状态冲突 | 复用既有 graphics 管线构建路径（`vulkan/pipeline.rs`），pass 间重设 viewport/scissor；RenderGraph 只管 barrier 不管录制顺序（沿用现状） |
| `TIMESTAMP_QUERY_COUNT` 固定容量不够多 pass | R-09 改为按 `build_frame_graph` pass 数动态分配 QueryPool（swapchain 重建同周期），容量不足时回退「只统计前 N 个 pass + warn」 |
| 校验层对多 set/新阶段映射报 VUID | `map_stages/map_access` 扩展时逐条对照 synchronization2 表；任何新增 `unsafe` 带 `SAFETY:` 注释 |

## 7. 不包含（移交后续里程碑）

| 项 | 去向 | 说明 |
|---|---|---|
| 异步纹理上传（GPU 侧延迟上传/传输队列） | M5 后小迭代 | 首版沿用阻塞传输路径，与 mesh 上传一致 |
| descriptor indexing / 绑定无界数组 | 需要时再做 | v0.3 材质 3 张纹理固定槽位即可 |
| SSAO 噪声 TAA 去噪、SSGI/DDGI、虚拟几何 | N3 | 研究型 |
| 多光源阴影（点光 cube shadow）、CSM 稳定性优化 | M5 后 | 单方向光 CSM 已满足 v0.3 |
| 纹理压缩格式（BC 系列）编码上传 | 随 M13 cook | KTX2 先做未压缩直通 |
| FLAC/FBX 导入 | 后续 | 与 M5 无关 |

## 8. 验收记录（执行后填写）

| 项 | 命令/方式 | 结果 | 日期 |
|---|---|---|---|
| 计划复核 | code-explorer 只读复核 | ✅ 16 条事实核对，改动点归纳 10 条 | 2026-11-27 |
| 格式/静态检查 | `cargo fmt --all -- --check`；`cargo clippy --workspace --all-targets -- -D warnings` | ✅ 通过（仅既有 Cargo 清单 unused dependency 警告） | 2026-11-27 |
| 单测 | `cargo test --workspace` | ✅ 全绿：mir-image 12 项（新增 9 项 KTX2）/ mir-render 65 项（新增 5 项纹理）+ 其余不回归 | 2026-11-27 |
| 纹理上传回归 | create/destroy 1000 次循环 + 校验层 | ✅ `headless_texture_create_destroy_stress` 1000 次无显存残留；`texture_upload_validation_layer`（ignored 手动触发）校验层通过、多 mip/非 2 幂尺寸/1×1 全覆盖 | 2026-11-27 |
| 窗口/headless/校验层 | `--frames 60`；`--headless --frames 1000 --no-validation`；`--validation --frames 120` | ✅ headless 1000 帧正常（批次 A 未接入示例画面，行为不变）；校验层 120 帧 0 error、58.3 FPS | 2026-11-27 |
| RenderGraph dump | `--graph-dump` 多 pass DOT 人工核对 | | |
| release 1080p | `--release --validation --frames 600 --width 1920 --height 1080 --stats`（对比 M4 基线 58.3 FPS） | | |
| IBL/CSM/SSAO/Bloom 效果对比 | 批次内画面对比 + `--stats` pass 耗时 | | |
| v0.3 演示 | GLB 场景加载 + 60 FPS 目标判定 | | |

## 9. 执行记录

### 2026-11-27：批次 0——计划与复核（已完成）

- code-explorer 复核确认：材质系统（48 B UBO + 世代句柄 + set1 仅 UBO）、描述符池无采样容量、
  全仓库无 sampler/彩色图像创建路径（depth.rs 为唯一模板）、分配器已支持图像分配、
  RenderGraph 无采样语义（`Usage` 4 种、`PassKind` 2 变体）、transient 别名已实现未使用、
  KTX2 整文件透传缺 mip 元数据、pbr.slang 光照为常量且无 module 机制、CI 双 job slangc 待扩充。
- 据此固化本计划：批次 A–G、Gate 1–7、关键技术决策 10 条、风险对策 8 条。

### 2026-11-27：批次 A——纹理基础设施（已完成，R-01/R-02/R-03）

- **R-01 KTX2/格式元数据**：`mir-image/src/ktx2.rs` 按规范头（80 B）完整解析——vkFormat/typeSize/
  pixelDepth/layerCount/faceCount/levelCount 与 levelIndex 逐 mip 范围（offset/length 越界校验）；
  vkFormat 白名单（37 UNORM / 43 SRGB，0 与压缩格式返回可读 `Unsupported`）；`Ktx2Data` 扩
  `mip_levels/array_layers/face_count/mip_data` 并加 `is_srgb()`。新增 9 项单测（截断头/层级越界/
  face 非法等负例先行）。
- **R-01 纯数据类型**：`mir-render/src/texture.rs` 落地公开 `ImageFormat`（6 变体：UNORM/SRGB/
  B10G11R11Ufloat/R16G16B16A16Sfloat/R32Sfloat/D32Sfloat，含 bytes_per_texel）、`TextureData`
  （构造时按 `max(1, dim >> i)` 逐 mip 校验尺寸/越界）、`MipRange`、`TextureHandle`（槽位+世代，
  对齐 MeshHandle/MaterialHandle）与 `TextureDataError`；`graph.rs` 的 crate 私有 `ImageFormat`
  删除并统一引用同一来源（消除双枚举漂移）。
- **R-02 GPU 路径**：`vulkan/texture.rs` 实现 `create_texture`（image[TRANSFER_DST|SAMPLED] →
  子分配器 DEVICE_LOCAL → bind → 视图（COLOR/DEPTH aspect）→ 采样器（LINEAR/REPEAT，
  maxLod = mip 数）→ staging 逐 mip `copy_buffer_to_image` → UNDEFINED→SHADER_READ_ONLY_OPTIMAL
  barrier → `queue_wait_idle` → staging 立即释放）与 `destroy_texture`（世代递增 + 槽位复用；
  有帧时经 DeletionQueue 绑定 frame fence，headless 立即释放）；`MAX_TEXTURES = 128` 槽位表。
  `DeletionEntry` 扩展 `images: Vec<DeletionImage>`（sampler/view/image + 显存），mesh/material
  构造点同步补空字段；`VulkanCore` 增槽位表字段与 Drop 收尾 `destroy_all_textures`。
- **R-02 门面**：`RenderInitError::Texture(String)` 变体 + `RenderContext::create_texture/
  destroy_texture`（对齐 mesh/material 语义）；re-export `ImageFormat/MipRange/TextureData/
  TextureDataError/TextureHandle`。
- **R-03 回归**：单测新增 `headless_texture_upload_roundtrip`（世代句柄/重复销毁/泄漏守恒）、
  `headless_texture_create_destroy_stress`（1000 次循环，分配器字节守恒断言）、
  `texture_upload_validation_layer`（ignored，校验层开启：64/33/1 多尺寸 + 守恒）。
  校验层实测 0 error。
- 决策偏差记录：KTX2 白名单收敛为未压缩 R8G8B8A8 UNORM/SRGB 两格式（压缩格式随 M13 cook，
  与 §7「不包含」一致）；`ImageFormat` 从 `graph.rs` 迁出为公开类型（原计划「统一」的实现方式）。
- Gate 结果：fmt/clippy -D warnings/test --workspace 全绿；headless 1000 帧行为不变；
  `--validation --frames 120` 0 error（58.3 FPS）。批次 A 人日消耗约 2.5（估算 3.0 内）。
