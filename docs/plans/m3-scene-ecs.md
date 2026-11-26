# M3 任务清单（场景与 ECS：Transform 统一 → hecs → Actor/层级 → 序列化 → 验收）

- 状态：**全部完成（批次 0–5 落地，M3 Gate 通过）——进入 M4**（2026-11-26）
- 上游依据：`README.md` §8.2 / §9.2 / §9.4 / §9.5 / §14.4；`docs/plans/m2-remaining.md` §10 末尾。
- 约束：Windows x64 + Vulkan 1.3（vulkanalia）、C# 脚本后续接入、物理仅接口、编辑器核心不依赖 UI；不引入第二图形后端或蓝图。

## 1. 目标与 Gate

**目标**：在保持 v0.2 演示画面与自由交互不退化的前提下，把空间数据、实体与场景对象模型升级到 M3 基线：

1. `Transform` 归 `mir-math`，单位统一为厘米、Z-up、左手系；`mir-scene` 保留兼容重导出。
2. `mir-ecs` 用 `hecs` 替换 `HashMap + dyn Any` 原型，提供世代句柄、组件查询、类型化 Resource 与基础变更检测。
3. `mir-scene` 提供 `ActorId`/`ActorMeta`/父子组件与 `Level`，世界矩阵在 extract 侧烘焙，不污染 `mir-render` 帧数据边界。
4. `mir-serialize` 提供版本化快照编解码；场景保存只记录稳定 ID、父子关系、局部变换与显式组件。
5. 示例迁移到新 API，验证保存→加载→再保存 golden 一致，父对象变换正确传递到子对象。

**Gate（全部满足才算 M3 达成）**：

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo run -- --frames 60`
- `cargo run -- --headless --frames 1000 --no-validation`
- `cargo run -- --validation --frames 120`，Vulkan 校验层 0 error
- `cargo run --release -- --validation --frames 600 --width 1920 --height 1080 --stats`，记录帧率并与 M2 基线 58.6 FPS 对比
- 10k 实体变换基准（扁平 + 纵深）记录机器与耗时，目标 ≤ 1 ms
- 场景保存、加载、再保存 golden 一致；父子公司循环、截断数据、版本不匹配均有失败测试

## 2. 现状核对（只读确认）

| 事实 | 位置 |
|---|---|
| M2 剩余批次 A–F 全部完成，文档明确进入 M3 | `docs/plans/m2-remaining.md` 第 4 行、§10 末尾 |
| `Transform` 当前定义在 `mir-scene`，并实现 `mir_ecs::Component` | `crates/mir-scene/src/lib.rs:36-61` |
| `mir-math` 已有 `Aabb/Frustum/Mat4::from_trs`，但无 `Transform`/`interop` | `crates/mir-math/src/lib.rs` 及 `frustum.rs`/`mat.rs` |
| `mir-ecs` 仍为 `Entity(u64)` + `HashMap<TypeId, HashMap<Entity, Box<dyn Any>>>` | `crates/mir-ecs/src/lib.rs:11-90` |
| `mir-serialize` 仍是空壳，依赖为空 | `crates/mir-serialize/src/lib.rs`、`Cargo.toml` |
| 应用层自行持有 `World`，`on_render` 读 `Transform` 并填 `InstanceData` | `crates/miriya-app/src/main.rs:19-24,272-312` |
| 当前示例数值仍是米级（阵列 ±3、scale 2、相机速度 5） | `crates/miriya-app/src/main.rs:54-111`、`crates/mir-scene/src/camera.rs` |
| `CameraFrame::camera_position` 注释仍写“米/单位与场景一致” | `crates/mir-render/src/frame.rs:16` |
| workspace 依赖集中在根 `Cargo.toml`，无 `[workspace.lints]`，CI 使用 `stable` | `Cargo.toml:14-18`、`.github/workflows/ci.yml` |

## 3. 任务总表

| 批次 | 任务 | 主要文件 | 依赖 | 验收 |
|---|---|---|---|---|
| 0 | 复核调用点并固化计划 | `docs/plans/m3-scene-ecs.md` | — | 本文件 |
| 1 | `Transform`/`interop` 下沉到 `mir-math`，统一厘米 | `mir-math`, `mir-scene`, `mir-render`, `miriya-engine` | 0 | 数学单测 + 示例可运行 |
| 2 | `mir-ecs` 换 `hecs`，世代句柄/Resource/变更检测 | `mir-ecs`, workspace 依赖 | 1 | crate 单测，API 兼容 |
| 3 | `Actor`/`Parent`/`Children`/`Level` 与层级世界变换 | `mir-scene` | 2 | 父子变换单测，10k 基准 |
| 4 | 版本化快照与场景往返 | `mir-serialize`, `mir-scene`, 集成测试 | 3 | golden + 错误输入测试 |
| 5 | 示例迁移与 Gate 验收、文档同步 | `miriya-app`, README, ADR | 4 | 全部 Gate |

## 4. 分阶段详述

### 批次 0：计划与复核

- 用 code-explorer 复核 `Transform`、`World`、`Scene`、示例 extract 的调用点与影响面。
- 固化 Gate、风险、执行记录到本文件。

### 批次 1：`Transform` 与单位统一

- `mir-math` 新增 `transform.rs`/`interop.rs`，导出 `Transform`、厘米/米转换常量与单位约定。
- `mir-scene` 删除本地 `Transform` 定义并重导出 `mir_math::Transform`；`Component` 实现由 `mir-ecs` 提供，避免数学层反向依赖。
- 为 `Vec/Quat/Mat4/Transform` 增加确定性 serde 表示；`CameraFrame` 注释改为厘米。
- 示例数值一次性改为厘米语义，不散落乘 100；保持相机近远裁剪与速度可用。

### 批次 2：`mir-ecs` 内核升级

- workspace 固定 `hecs`、`serde`、`postcard`、`thiserror`、`criterion` 版本。
- `mir-ecs` 拆分为 `entity/world/resource/change/error`；`Entity` 为世代句柄 newtype。
- `World::spawn/despawn/insert/get/get_mut/query` 保持门面形态，错误返回 `EcsError`；提供 `Resource` 容器与 Added/Changed 基础能力。

### 批次 3：Actor 与层级

- `ActorId`/`ActorMeta`/`Parent`/`Children` 以显式组件表达；`Level` 持有 ECS World 与 Actor 索引。
- 世界矩阵以拓扑序单次遍历计算，父子链循环检测并报错；extract 侧用世界矩阵填 `InstanceData`。
- 新增 10k 实体基准（扁平与纵深），记录耗时。

### 批次 4：序列化与 golden

- `mir-serialize` 实现 magic/version/schema hash 头与 `encode/decode`；损坏数据与版本不匹配返回可读错误。
- `mir-scene` 实现 `ActorSnapshot/SceneSnapshot`，只保存稳定 ID、父级、局部变换与显式组件；加载后重建实体并重算世界变换。
- 新增跨 crate 集成测试：保存→加载→再保存 golden；循环父子、截断数据、版本不匹配负例。

### 批次 5：示例迁移与验收

- `miriya-app` 迁移到 `Level`/`Actor` 与世界矩阵；保留 3×3 阵列、中心圆环、两枚球和自由相机行为。
- 执行全部 Gate，记录窗口/headless/校验层/release 1080p 结果与 10k 基准。
- 更新 README §6.3/§9.2/§9.4/§9.5/§14、ADR-0002/0003/0012 与本文件执行记录。

## 5. 依赖顺序

`计划 → Transform/单位 → ECS → Actor/层级 → 序列化 → 示例/Gate`。每一批完成后先跑 `cargo test --workspace` 与 `cargo clippy --workspace --all-targets -- -D warnings`，再进入下一批。

## 6. 风险与对策

| 风险 | 对策 |
|---|---|
| `Transform` 下沉导致 `mir-math` 反向依赖 ECS | 组件标记在 `mir-ecs` 侧统一 impl；`mir-math` 保持只依赖 `glam`/serde |
| 厘米校准遗漏导致画面异常或视锥剔除错误 | 集中在 `interop` 与示例常量；extract 用世界空间 AABB；窗口/headless/校验层验收 |
| hecs 替换破坏既有 API | 门面保留 `spawn/get/insert/query` 形态；应用层集中迁移；crate 单测覆盖失效句柄 |
| 层级循环或脏子树导致死循环/错误矩阵 | 显式循环检测并报错；拓扑序遍历；父子链单测与负例测试 |
| 序列化保存 ECS 内部实体编号导致不可复现 | 只保存 `ActorId`；加载时重建映射并重算世界变换 |
| 新增依赖与基准影响 CI | 依赖写入 workspace 统一版本；基准若只编译，`--all-targets` 已覆盖；不新增运行时依赖 |

## 7. 不包含（明确留待后续里程碑）

| 项 | 去向 | 说明 |
|---|---|---|
| glTF / 纹理 / 音频资产导入 | M4 | 示例仍用程序化几何 |
| IBL/CSM/SSAO/Bloom/多光源 | M5 | 维持当前 PBR 单方向光 |
| 物理求解器与固定步胶水 | M6 | `mir-physics` 仍只有契约与 Null 后端 |
| C# 脚本宿主与绑定 | M8 | .NET 运行时仍未接入 |
| 编辑器 UI 后端选定 | M12 | `mir-editor-core` 仅做语义对齐，不新增 UI 依赖 |
| 高频场景重建的多线程录制 | M3 之后 | 帧录制仍在主线程 |

## 8. 验收记录（执行后填写）

| 项 | 命令/方式 | 结果 | 日期 |
|---|---|---|---|
| 计划复核 | code-explorer 只读复核 | ✅ 调用点确认，风险已登记 | 2026-11-26 |
| 格式/静态检查 | `cargo fmt --all -- --check`；`cargo clippy --workspace --all-targets -- -D warnings` | ✅ 通过（仅既有 Cargo 清单 unused dependency 警告，未升级为错误） | 2026-11-26 |
| 单测 | `cargo test --workspace` | ✅ 全部 crate 通过，新增 mir-math 22 / mir-ecs 4 / mir-scene 10 / mir-serialize 1 / 集成 4 项 | 2026-11-26 |
| 窗口 60 帧 | `cargo run -- --frames 60` | ✅ 12 Actor / 11 提交实例，平均 56.5 FPS；管线缓存写回因本机权限 warn（非 M3 回归） | 2026-11-26 |
| headless 1000 帧 | `cargo run -- --headless --frames 1000 --no-validation` | ✅ 退出正常 | 2026-11-26 |
| Vulkan 校验层 | `cargo run -- --validation --frames 120` | ✅ 0 validation error，平均 58.1 FPS | 2026-11-26 |
| release 1080p | `cargo run --release -- --validation --frames 600 --width 1920 --height 1080 --stats` | ✅ 600 帧 58.2 FPS（M2 基线 58.6 FPS），提交 11 实例，`main_geometry` 0.071 ms，0 validation error | 2026-11-26 |
| 10k 基准 | `cargo bench -p mir-scene --bench hierarchy` | ✅ 扁平 0.101 ms / 纵深 0.356 ms（release，20 样本），目标 ≤ 1 ms | 2026-11-26 |
| 场景 golden | `cargo test -p miriya-integration-tests` | ✅ 保存→加载→再保存逐字节一致；父子公司循环、截断数据、版本不匹配均失败 | 2026-11-26 |

## 9. 执行记录

### 2026-11-26：批次 0 完成

- 复核结论：`Transform` 仅 `mir-scene` 定义，`miriya-app` 为唯一真实调用方；`World::query` 当前无调用点；workspace 依赖集中在根 `Cargo.toml`；CI 未跑 `cargo bench`，但 `clippy --all-targets` 会检查基准代码。
- 据此确认实施顺序：空间数据 → ECS 内核 → 场景对象模型 → 序列化 → 示例与验收。

### 2026-11-26：批次 1——Transform 与单位统一（已完成）

- `mir-math` 新增 `transform.rs`/`interop.rs`，`Transform` 归数学层并派生稳定 serde；`mir-scene` 保留重导出，
  `mir-ecs` 负责组件标记；`Vec/Quat/Mat4` 用数组代理实现确定性 serde。
- 相机 near/far 1 cm / 1000 m、速度 500 cm/s；示例数值经 `interop` 一次性转换，画面/视锥剔除保持一致；
  `CameraFrame::camera_position` 注释改为厘米。
- 验证：`cargo test -p mir-math -p mir-scene`、`cargo run -- --headless --frames 60 --no-validation` 通过。

### 2026-11-26：批次 2——mir-ecs 换 hecs（已完成）

- `hecs 0.10` 包成门面：世代 `Entity`、`Ref`/`Mut` 守卫、回调式查询、`Resources`、`ChangeTracker` 与 `EcsError`。
- 适配 hecs 0.10 的 `get::<&T>()` / `get::<&mut T>()` / `QueryBorrow::iter()` API；失效句柄、变更分类与资源隔离均有单测。
- 验证：`cargo test -p mir-ecs` 通过，示例继续 11 实例提交。

### 2026-11-26：批次 3——Actor 与层级（已完成）

- `mir-scene` 落地 `ActorId`/`ActorMeta`/`Parent`/`Children`/`Level`；世界矩阵按拓扑序单次遍历，
  父子链循环检测；`Aabb::transformed` 支撑世界空间剔除。
- 性能：初版 HashMaps 4.7 ms → 平行数组 2.2 ms → Level 紧凑缓存 0.10/0.36 ms；
  直接修改组件需走 Level API 以同步缓存（`world_mut()` 已注明约束）。
- 验证：父子平移/旋转、循环拒绝、解除父子关系 3 项单测通过；10k 基准达标。

### 2026-11-26：批次 4——版本化快照与 golden（已完成）

- `mir-serialize` 实现 magic/version/schema hash 头 + postcard 编解码；`mir-scene` 实现
  `SceneSnapshot`/`ActorSnapshot`，只保存稳定 ID、父子关系、局部变换与元数据。
- 新增 workspace 成员 `tests`（`miriya-integration-tests`），覆盖保存→加载→再保存 golden、截断、版本不匹配与循环快照。
- 验证：`cargo test -p miriya-integration-tests` 4/4 通过。

### 2026-11-26：批次 5——示例迁移与 Gate（已完成）

- `miriya-app` 改用 `Level`/`ActorId`；8 个立方体挂在恒等根 Actor 下验证层级链路而不改变画面；
  extract 用世界矩阵与变换后 AABB，继续输出 `CameraFrame`/`InstanceData` 纯数据。
- 全部 Gate 已执行并记录于 §8。下一步：M4 资产管线（异步加载、导入器、错误化与 `.miasset`）。
