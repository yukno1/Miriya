# M4 任务清单（资产管线：注册表 → 容器 → 导入 → 异步/热重载 → 验收）

- 状态：**全部完成（R-01…R-06 落地，M4 Gate 通过）——进入 M5**（2026-11-26）
- 上游依据：`README.md` §8.2 / §9.7 / §10 / §14.4；`docs/plans/m3-scene-ecs.md` 末尾“进入 M4”。
- 约束：Windows x64 + Vulkan 1.3（vulkanalia）；C# 宿主 M8；物理仅接口；编辑器核心不依赖 UI；不引入第二图形后端或蓝图。

## 1. 目标与 Gate

1. `mir-asset` 从同步 `panic`/裸 `u64`/`dyn Any` 原型升级为 `AssetId`(UUIDv7) + `AssetError` + 注册表/缓存分离。
2. 实现 `.miasset`：magic/version/schema hash + 分块 zstd + checksum；损坏输入与版本不匹配返回可读错误。
3. 实现 glTF（GLB）网格/材质、PNG/KTX2 纹理、WAV/OGG 音频导入；坐标/单位转换集中在 `mir-math::interop`。
4. 实现异步加载与 `notify` 热重载：主线程只 `poll()` 完成队列，依赖反向闭包按拓扑序失效。
5. 示例保持程序化回退，资产可用时走 `.miasset` 加载路径；画面/free-camera/PBR/校验层不退化。

**Gate**：
- `cargo fmt --all -- --check`；`cargo clippy --workspace --all-targets -- -D warnings`；`cargo test --workspace`。
- 窗口 60 帧、headless 1000 帧、校验层 120 帧、release 1080p 600 帧与 M3 基线基本一致。
- 容器 golden、损坏输入、版本不匹配、glTF 单位转换、热重载耗时与 1 GB 冷加载主线程阻塞基准可复现。

## 2. 复核结论（2026-11-26 code-explorer）

- `mir-asset` 当前仅被 `mir-core::EngineContext.assets` 与门面 re-export 引用；无 `impl Asset`/`load`/`get` 调用点，重构风险集中在 API 设计而非兼容调用方。
- `EngineContext` 未调用 `set_root`；`tick()` 与 `run_headless()` 均无资产 poll/shutdown 钩子，M4 需补入。
- `mir-render` 只有 `MeshData`/`MaterialData` 纯数据入口，无纹理采样/上传 API；资产层只生成纯数据，GPU 创建仍由应用层完成（ADR-0012）。
- `tools/miriya-cli` 与 `tools/mir-build` 均为空壳；CI 唯一文件 `.github/workflows/ci.yml`，当前不调用资产工具。
- 当前 `assets/` 无内容；导入器需显式相对 `AssetServer::root()` 解析路径，避免依赖 CWD。

## 3. 任务总表

| ID | 任务 | 依赖 | 估时 | Gate |
|---|---|---|---:|---|
| R-01 | 计划文档与 ADR-0013 | — | 0.5 d | 文档落盘 |
| R-02 | AssetId/AssetError/Asset trait/Registry/DependencyGraph/Cache | R-01 | 2 d | 单测 + 无 panic 路径 |
| R-03 | `.miasset` 容器与 zstd/checksum | R-02 | 1.5 d | golden + 损坏输入 |
| R-04 | glTF/PNG/KTX2/WAV/OGG 导入 + interop | R-02,R-03 | 3 d | 导入报告 + 单位测试 |
| R-05 | rayon 异步加载 + notify 热重载 + poll/shutdown | R-04 | 2 d | ≤ 200 ms 热重载数据 |
| R-06 | CLI cook/mir-build + 示例接入 + CI | R-05 | 1.5 d | 全 Gate |

## 4. 关键设计

- **注册表/缓存分离**：`AssetRegistry` 只存 `AssetMeta{id,path,type_tag,hash,deps}`；内容缓存独立，按 `AssetId` 索引。
- **句柄**：`Handle<A>` 携带 `AssetId` 与 `PhantomData<A>`；`Asset::Native` 关联类型决定 `get::<A>()` 返回的具体纯数据类型。
- **错误**：`AssetError::{NotFound,InvalidPath,Io,Corrupt,Version,Decode,Watch,Unsupported,Cycle,MissingDependency,Busy}`，所有解析路径返回 `Result`。
- **容器**：`MIRI` magic + version/schema hash + 64 KiB 分块 zstd + FNV checksum；解码线性扫描，坏块隔离。
- **依赖图**：正向依赖用于 cook 闭包，反向依赖用于热重载失效；失效顺序拓扑排序，循环返回 `AssetError::Cycle`。
- **异步**：`rayon::spawn` 做 IO/解码/哈希，`mpsc` 回传；`AssetServer::poll()` 由 `mir-core` 每帧调用，headless 同样调用。
- **热重载**：`notify` 事件 → 相对路径 → 内容哈希比对 → 重提交作业；`reload_latency_ms()` 暴露最近一次耗时供 Gate 验证。

## 5. 不包含

| 项 | 去向 | 说明 |
|---|---|---|
| IBL/CSM/SSAO/Bloom/多光源 | M5 | 保持单方向光 PBR |
| 纹理采样/材质纹理绑定 | M5 | 本 M4 只生成 `ImageData` 纯数据 |
| 物理接口胶水 | M6 | `mir-physics` 仍仅契约 |
| 作业调度 `Schedule` | M7 | 资产作业池独立，不接入系统调度器 |
| C# 脚本与绑定 | M8 | .NET 运行时未接入 |
| 编辑器 UI 与内容浏览器 | M12 | 仅提供 CLI cook 与 headless 验收 |

## 6. 验收记录（执行后填写）

| 项 | 命令/方式 | 结果 | 日期 |
|---|---|---|---|
| 计划复核 | code-explorer 只读复核 | ✅ 调用点与风险已确认 | 2026-11-26 |
| 格式/静态检查 | `cargo fmt --all -- --check`；`cargo clippy --workspace --all-targets -- -D warnings` | ✅ 通过（仅既有 Cargo 清单 unused dependency 警告） | 2026-11-26 |
| 单测 | `cargo test --workspace` | ✅ 全绿：mir-asset 16 项 / mir-image 3 项 / 集成 asset_pipeline 4 项 + scene_roundtrip 4 项 | 2026-11-26 |
| 窗口/headless/校验层 | `cargo run -- --frames 60`；`--headless --frames 1000 --no-validation`；`--validation --frames 120` | ✅ 窗口 58.0 FPS、headless 1000 帧正常、校验层 0 error；示例日志确认 cooked 资产异步就绪 | 2026-11-26 |
| release 1080p | `cargo run --release -- --validation --frames 600 --width 1920 --height 1080 --stats` | ✅ 600 帧 58.3 FPS（M3 基线 58.2 FPS），提交 11 实例，`main_geometry` 0.071 ms，0 validation error | 2026-11-26 |
| 容器 golden/损坏输入 | `cargo test -p miriya-integration-tests asset_pipeline` | ✅ cook 字节可复现；截断/checksum/版本不匹配均返回可读错误 | 2026-11-26 |
| 热重载 ≤ 200 ms | `cargo test -p mir-asset server::tests::reload_records_latency_under_budget` | ✅ 作业耗时 < 200 ms，依赖链失效顺序有单测 | 2026-11-26 |
| 1 GB 冷加载基准 | `MIRIYA_ASSET_BENCH_1GB=1 cargo test -p miriya-integration-tests cold_load_poll_benchmark -- --ignored --nocapture` | ✅ 1024 资产 ≈ 1024 MB，总耗时 478 ms，主线程单次 poll 最大 10.174 ms（≤ 16 ms） | 2026-11-26 |

## 7. 执行记录

### 2026-11-26：批次 0——计划复核（已完成）

- code-explorer 确认 `mir-asset` 旧 API 无实际调用点；`EngineContext` 无 poll/shutdown 钩子；`mir-render` 只有 `MeshData`/`MaterialData` 纯数据入口。
- CI 唯一文件 `.github/workflows/ci.yml`，未调用资产工具；`assets/` 仅有占位，导入器需显式相对根路径解析。

### 2026-11-26：批次 1——ID/错误/注册表/依赖图（已完成）

- 落地 `AssetId`(UUIDv7)、`AssetError`、`Asset` 关联类型契约、类型化 `Handle<A>`、`AssetRegistry`（路径归一化与元数据）、
  `DependencyGraph`（cook 正序与热重载反向失效拓扑）。
- 修复初版 `invalidation_order` 误用 cook 反序的问题：改为沿反向边 DFS 的依赖者优先顺序。
- 验证：`cargo test -p mir-asset` 核心单测通过。

### 2026-11-26：批次 2——`.miasset` 容器（已完成）

- `MIRI` magic/version/schema hash + 64 KiB 分块 zstd + FNV checksum；版本不匹配、截断、checksum 损坏均返回可读错误。
- 本机 MSVC 18 在 `zstd-sys` 的 `/Z7` 下触发 D8050；通过 workspace profile 仅对该 C 依赖关闭调试信息规避，不影响引擎 debug 构建。
- 验证：3 项容器单测 + 集成 golden 通过。

### 2026-11-26：批次 3——导入器（已完成）

- GLB 网格/PBR 材质（`gltf` crate）、PNG/KTX2（`mir-image`）、WAV（手写 PCM 解析）与 OGG（symphonia）导入。
- glTF Y-up/米 → Z-up/厘米只在 `mir-math::interop`；导入报告记录转换节点与警告。
- 验证：导入器负例与 WAV 正例单测通过。

### 2026-11-26：批次 4——异步加载与热重载（已完成）

- rayon 作业池执行 IO/哈希/解码/解压；`mpsc` 回传完成队列；`AssetServer::poll()` 主线程结算。
- notify 事件 → 相对路径 → 内容哈希 → 反向依赖闭包重提交；`reload_latency_ms()` 可观测。
- `mir-core` 主循环与 headless 均调用 `assets.poll()` / `assets.shutdown()`。
- 验证：异步加载、缺失资产失败态、热重载耗时、监听器根路径校验 4 项单测通过。

### 2026-11-26：批次 5——工具链/示例/Gate（已完成）

- `mir-build::cook_file/cook_dir` 先解析校验再封装 `.miasset`；`miriya-cli cook` 可复现产物；CI 增加 cook golden 校验。
- 示例在 `on_start` 请求 `demo/tone.miasset`，异步就绪后输出日志；程序化几何作为回退路径保持画面不变。
- 全部 Gate 已执行并记录于 §6。下一步：M5 材质与光照（IBL/CSM/SSAO/Bloom 与纹理 GPU 上传）。
