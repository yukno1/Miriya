# Miriya

> **用 Rust 重写的 Unreal Engine —— Windows + Vulkan 的独立实现子集，玩法使用 C#**
>
> 目标不是把上游 8 万多个 C++ 文件逐行翻译成 Rust，而是以 UE 的模块划分、编辑器工作流与运行时语义为参照，
> 构建一个现代、内存安全、无 GC 停顿、可由小团队长期维护的 3D 引擎与工具链。

| 项目信息 | 说明 |
|---|---|
| 上游基线 | Unreal Engine **5.8.4**（同机目录 `../UnrealEngine`，仅作行为参考） |
| 当前阶段 | **M2 · v0.2 Cube 已达成**（渲染 v0 剩余工作仍在进行） |
| 目标平台 | **Windows x64（Vulkan 1.3）**；Linux 接口保留，暂不发布 |
| 图形后端 | **Vulkan 1.3，唯一后端（vulkanalia）**；不使用 wgpu / ash / DX12 / OpenGL |
| 玩法脚本 | **C#（.NET 11）**；不使用蓝图，不使用 WASM 脚本 |
| 物理 | **仅提供接口（`mir-physics`）**；不实现 Chaos/Rapier 等求解器，由外部后端注入 |
| 编辑器 | **核心与 UI 框架解耦**（`mir-editor-core`）；GPUI 为候选，egui 可选 |
| 迁移策略 | **不做 1:1 移植**；语义对齐，优先使用 Rust 语言特性（见 §1.3 / §4.2） |
| 许可 | MIT OR Apache-2.0（拟定，见 §11.6） |
| 文档状态 | 规划文档，随里程碑更新。标记：✅ 可用 · 🟡 原型 · ⬜ 规划 |

**目录**：[1. 项目定位](#1-项目定位) · [2. 上游基线](#2-上游基线) · [3. 范围界定](#3-范围界定) ·
[4. 技术路线与关键取舍](#4-技术路线与关键取舍) · [5. 架构设计](#5-架构设计) ·
[6. 仓库结构与-crate-规划](#6-仓库结构与-crate-规划) · [7. UE 模块映射](#7-ue-模块映射) ·
[8. 里程碑与路线图](#8-里程碑与路线图) · [9. 子系统设计要点](#9-子系统设计要点) ·
[10. 资产与数据格式](#10-资产与数据格式) · [11. 工程规范与质量保障](#11-工程规范与质量保障) ·
[12. 风险与应对](#12-风险与应对) · [13. 验收标准](#13-验收标准) ·
[14. 当前进度与快速开始](#14-当前进度与快速开始) · [15. 参考与术语](#15-参考与术语) · [16. 如何参与](#16-如何参与)

> **本次修订（2026-11-18）**：① 平台收敛为 **Windows-only + Linux 接口预留**；② 渲染唯一后端改为
> **Vulkan 1.3（`vulkanalia`）**；③ 玩法脚本改用 **C#（.NET）**，彻底移除蓝图/可视化脚本/WASM 方案；
> ④ **不实现物理引擎**，仅保留 `PhysicsBackend` 接口供外部自研后端接入；⑤ **编辑器与 UI 框架解耦**，
> 核心逻辑不依赖 egui/GPUI，GPUI 作为候选后端；⑥ **不要求 1:1 迁移**，公共 API 与内部设计优先采用
> Rust 语言特性。对应设计见 §1.3、§3.5、§4.2、§5.5–§5.7、§9.6、§9.9、§9.15。

---

## 1. 项目定位

**一句话**：让一个小团队能够完成「导入资产 → 搭关卡 → 写玩法 → 打包发行」的完整闭环，并在关键路径上对齐
Unreal Engine 的开发者体验，同时获得 Rust 的工程收益。

### 1.1 三个原则

1. **独立实现（clean-room）**：只参考 UE 的公开文档、模块划分与可观察行为，不复制、不逐行改写其源代码，
   不引入其私有实现细节；所有代码以 Rust 语义重新设计（见 §11.6）。
2. **垂直切片（vertical slice）**：每个里程碑都以「一个能跑、能验收的 demo」结束，而不是先造五个半成品子系统。
3. **渐进对齐（progressive parity）**：先对齐工作流与语义（Actor/Component、材质、PIE、打包），
   再逐项替换为更适合 Rust 的实现（ECS、异步资产、引擎对象不依赖 tracing GC；C# 脚本侧由 .NET GC 管理）。

### 1.2 为什么不做 1:1 迁移

- 上游 `Engine/Source` 有 **80,146** 个 `.cpp/.h/.cs/.usf/.ush` 文件、约 **984 MB** 文本；仅 Runtime 就有
  **194** 个模块、**18,544** 个 C++ 文件（详见 §2.2）。完整对齐（含 Lumen/Nanite/Chaos、主机平台、
  蓝图生态与编辑器全部工具）远超本项目的资源边界——UE 由数百名工程师持续开发了二十年以上。
- Rust 的优势在内存安全、并发、包管理与工具链；劣势在编辑器、着色器工具链、主机 SDK 与内容生态。
  因此计划**主动裁剪范围、拉长战线、以可玩的垂直切片为验收**，而不是宣称「Rust 版 UE 一夜完成」。
- 1:1 迁移既无必要也不划算：C++ 的类层次、UObject 布局、头文件/宏、反射代码生成与 API 命名，
  大多是为 C++ 生态和历史包袱服务的；Rust 有更合适的表达方式（见 §1.3）。
- Miriya 与 Epic Games 无关联；项目名称、文档与产物均不使用 Unreal/UE 商标作为产品名。

### 1.3 不追求 1:1：Rust 优先

- **"1:1" 的边界**：不要求类名/文件名/头文件/对象布局/虚表结构/调用栈/蓝图字节码与 UE 对齐；
  UE 只提供行为、术语与工作流的参考。允许 Miriya 的公共 API 与 UE 命名不同。
- **重写而非翻译**：用所有权与借用、RAII、newtype、`enum` + `match`、trait/泛型、`Result`/`Option`、
  迭代器、derive 宏、feature flag 与零成本抽象来重新设计系统，而不是把 C++ 结构"翻译"成 Rust 语法。
- **拒绝的 C++ 习惯**：深继承的 `UObject` 层级、手工虚表、`get_/set_` 满天飞、裸指针所有权、
  异常作控制流、全局可变单例、用宏代替泛型、`Any`/`dyn` 到处做类型擦除（只在必要边界使用）。
- **验收看质量而非形似**：类型安全、无数据竞争、错误可恢复、依赖方向可编译验证、API 符合 Rust 惯用法，
  而不是"每个 UE 文件都能找到对应 Rust 文件"。
- **语言边界**：Rust 用于引擎、工具、编辑器核心与各子系统接口；C# 只负责玩法脚本（§9.13），
  不侵入引擎实现；UI 框架（GPUI/egui）只是编辑器适配层。
- **稳定优先**：只使用稳定版 Rust 特性；不为"炫技"引入 nightly-only 能力（fuzzer/bench 等工具链除外）。

---

## 2. 上游基线

### 2.1 目录构成

上游 `../UnrealEngine`（UE 5.8.4）的顶层结构：

| 路径 | 内容 | 对本项目的意义 |
|---|---|---|
| `Engine/Source/{Runtime,Editor,Developer,Programs,ThirdParty}` | C++ 模块、工具与第三方库 | 模块划分与 API 语义的主要参考 |
| `Engine/Build` | UBT 构建脚本、版本信息 | 构建/打包流程的参考 |
| `Engine/Config` | `Base*.ini` 配置体系 | 配置项语义参考（Miriya 用 TOML） |
| `Engine/Shaders/{Private,Public,Shared}` | HLSL 着色器与 `.ush` 公共库 | 材质/光照管线的语义参考（Miriya 用 Slang/SPIR-V） |
| `Engine/Platforms` + `Binaries` | 平台抽象与预编译二进制 | 平台支持范围的参照 |
| `Engine/Plugins` | 76 个一级插件目录 | 插件化架构的参照 |
| `Samples/Games/Lyra`、`Templates`（37 个模板） | 示例与游戏模板（第一/第三人称等） | **v1.0 的验收蓝本**（TP_FirstPerson / TP_ThirdPerson） |
| `Engine/Source/Programs` | UBT、UHT、UnrealPak、Insights 等 | 工具链的替代清单（见 §7） |

### 2.2 规模测量（本机 checkout）

| 范围 | 文件数 | 文本体积 |
|---|---:|---:|
| `Engine/Source` 全部（`.cpp/.h/.cs/.usf/.ush`） | 80,146 | 984.0 MB |
| `Engine/Source/Runtime`（`.cpp/.h`） | 18,544 | 215.2 MB |
| `Engine/Source/Editor`（`.cpp/.h`） | 8,180 | 75.9 MB |
| `Engine/Source/Developer`（`.cpp/.h`） | 3,374 | 30.9 MB |
| `Engine/Source/Programs`（`.cpp/.h`） | 1,818 | 18.2 MB |
| `Engine/Source/ThirdParty`（`.cpp/.h`） | 42,706 | 595.5 MB |

模块数量：Runtime **194**、Editor **145**、Developer **138**、Programs **104**、Plugins **76**、Templates **37**。

单个模块规模（`.cpp/.h`）：`Engine` 4,007 文件 / 54.7 MB；`Core` 2,105 / 21.6 MB；`Renderer` 768 / 20.0 MB；
`CoreUObject` 956 / 12.5 MB；`RHI` 98 / 2.2 MB；`RenderCore` 205 / 3.6 MB；`Slate` 464 / 4.6 MB；
`Net` 444 / 4.3 MB；`AudioMixer` 77 / 1.2 MB；`PhysicsCore` 64 / 0.4 MB；`UnrealEd` 1,541 / 19.3 MB。

> 结论：**上游体量决定了本项目必须做减法**。UE 的价值在于完整工作流与内容管线，Miriya 的价值在于
> 用 Rust 重建其中「可玩、可维护、可扩展」的那条主线。

---

## 3. 范围界定

### 3.1 三个同心环

| 环 | 版本 | 必须交付 | 明确不含 |
|---|---|---|---|
| **环 1：可玩闭环** | v1.0 | 桌面 3D 运行时、PBR 渲染、物理接口（外部后端）、动画、音频、输入、基础 HUD/菜单 UI、轻量编辑器、打包 | Lumen/Nanite 级渲染、主机、移动端、网络、富 UI 框架 |
| **环 2：生产扩展** | v1.x | Linux 支持、关卡流式、网络复制、富运行时 UI（数据绑定）、高级动画 | 与 UE 二进制/资产格式互通 |
| **环 3：研究项** | 不定 | GI（DDGI/SSGI）、虚拟几何（meshlet）、音频图、重定向、路径追踪实验 | 对外承诺的发布日期 |

### 3.2 v1.0 必须做到（环 1）

- **运行时**：窗口/输入、场景与 Actor 模型、变换层级、异步资产、PBR（含 IBL、CSM、SSAO、Bloom）、
  **物理接口与外部后端接入（Miriya 不实现物理引擎）**、骨骼动画（状态机 + 混合）、空间音频、HUD/菜单、
  保存/加载、**C# 玩法脚本运行时（编译、加载、热重载）**。
- **工具**：`miriya new/run/build/cook/pack` 命令；轻量编辑器（视口、拾取、Gizmo、Inspector、内容浏览器、
  撤销/重做、PIE）；glTF/PNG/KTX2/WAV/OGG 导入；C# 项目模板与绑定生成。
- **质量**：Windows 发布 + Linux 接口编译检查；单元/集成/golden 测试；基准与性能预算；崩溃可诊断；资产解析 fuzz。
- **蓝本**：用 Miriya 复刻 UE `TP_FirstPerson` / `TP_ThirdPerson` 的核心玩法与工作流（不复制其内容）。

### 3.3 明确不做（Non-goals）

- ❌ 100% 的 UE C++ API、ABI、蓝图字节码、`.uasset/.umap` 运行时兼容。
- ❌ 主机平台（PS/Xbox/Switch）与需要 NDA SDK 的集成。
- ❌ UE 编辑器全功能对齐（Sequencer、Niagara、MetaHuman、虚拟制片工具链）以及任何蓝图/可视化脚本系统。
- ❌ UE C++/蓝图插件二进制兼容；Miriya 的扩展单位是 Rust crate 与 C# 程序集。
- ❌ 任何物理引擎实现（Chaos、Rapier、PhysX 等）：Miriya 只提供 `PhysicsBackend` 接口，求解器由外部后端提供。
- ❌ 当前的 Linux/macOS/移动端/主机发布：Linux 只保留接口，其余平台不做（见 §3.5）。
- ❌ 首发即支持 VR/AR（不做）与联网多人（仅列入环 2）。
- ❌ 与 UE 逐帧一致的渲染输出（Lumen/Nanite/TAA 细节不同属正常）。

### 3.4 兼容性策略

- **自有格式优先**：运行时只读 Miriya 原生格式（见 §10）；导入器负责把 glTF/PNG/KTX2/音频转成原生资产。
- **可选只读导入**：`mir-uasset`（feature-gated）实验性解析 UE 包，用于迁移参考与内容盘点；
  不承诺保真，不复制其序列化实现，法律与 EULA 风险见 §11.6。
- **不承诺互操作**：Miriya 项目不能直接用 UE 打开，UE 项目也不直接由 Miriya 加载。

### 3.5 硬约束

1. **平台**：当前只支持 **Windows x64**。Linux 不发布，但窗口/输入/文件/线程/音频设备等平台层必须保留
   Linux 接口（trait + 可编译的占位实现 + CI 交叉检查），禁止把 Windows 专有类型（`HWND`、`HINSTANCE` 等）
   暴露到公共 API；后续启用 Linux 时只替换平台后端，不重写子系统。
2. **图形**：唯一渲染后端是 **Vulkan 1.3，通过 `vulkanalia`**。不引入 wgpu、ash、DX12、OpenGL、Metal，
   也不做第二套 RHI 抽象；Linux 后续复用同一套 Vulkan 渲染代码。
3. **脚本**：**不使用蓝图，也不做任何可视化脚本/节点图系统**。游戏逻辑与创作代码使用 **C#（.NET 11）**，
   引擎侧通过版本化 C ABI 与生成绑定对接；Rust 负责引擎与性能敏感路径。
4. **C# 的边界**（本计划的默认解释）：C# 用于**游戏逻辑与编辑器中的游戏代码编辑**；编辑器外壳是
   Rust + **可插拔 UI 后端**（见 §5.7 / §9.15），不绑定 egui 或 GPUI。若要求编辑器 UI 也用 C#，需另立方案。
5. **物理**：不实现任何物理引擎（Chaos/Rapier/PhysX 等都不做）。`mir-physics` 只提供 `PhysicsBackend`
   接口、固定步调度、句柄/查询/调试绘制契约；刚体、碰撞、约束、角色控制器与求解器由外部自研后端提供，
   Miriya 仓库内不含求解器代码。
6. **编辑器分层**：`mir-editor-core` 不得依赖 egui、GPUI、winit、Vulkan 等 UI/图形/窗口类型；
   UI 后端可插拔（GPUI 候选、egui 备选、headless 用于测试）。换 UI 框架不需要改编辑器核心逻辑。
7. **不做 1:1 移植**：不要求类/文件/头文件/布局/ABI 与 UE 对齐；公共 API 优先 Rust 惯用法与类型安全，
   允许与 UE 命名不同（见 §1.3、§4.2）。

---

## 4. 技术路线与关键取舍

### 4.1 技术取舍

> 下表是**职责与语义映射**，不是文件/类型的一一对应迁移。Miriya 的公共 API、错误模型、对象生命周期与
> 模块边界均按 Rust 惯用法重新设计（见 §4.2），允许与 UE 命名/结构不同。

| UE 机制 | 上游复杂度 | Miriya 方案 | 理由与代价 |
|---|---|---|---|
| `UObject` + UHT 代码生成反射 | 956 文件 / 12.5 MB | `#[derive(Reflect)]` 过程宏 + `TypeRegistry`（`mir-reflect`）；不模拟 UObject 继承/布局 | 无独立代码生成步骤，编译期即得元数据；代价是宏诊断与增量编译需打磨 |
| Tracing GC（引擎对象） | 停顿与跨线程复杂 | `slotmap` 世代句柄 + 显式 `destroy` + `WeakHandle` | 确定性、无 STW；代价是环引用需手工弱化（C# 侧仍由 .NET GC 管理） |
| Blueprint 虚拟机 | 庞大的字节码/节点图生态 | **完全不实现蓝图**；游戏逻辑用 C#（.NET 11）+ 版本化 C ABI | 无节点图与字节码负担，C# 生态成熟；代价是跨语言边界、GC 与热重载复杂度 |
| C# 脚本层 | UE 无对应（UBT 用 C# 但不参与运行时） | `mir-script`：hostfxr 托管 .NET 11、生成绑定、可卸载 ALC 热重载 | 语言成熟、工具链完整；代价是 .NET 运行时依赖与互操作测试 |
| Actor/Component + World | `Engine` 4,007 文件 | Actor 为「带层级/反射元数据的实体」，数据组件进 ECS；不做 C++ 继承/虚表 | 缓存友好、易并行；代价是保留一层 Actor 兼容外观 |
| Chaos / 物理引擎 | 大规模自研 | **不实现**；`mir-physics` 只提供 `PhysicsBackend` 接口，外部后端注入 | 引擎不背求解器维护成本，用户可用自研后端；代价是后端契约、集成与性能由外部负责 |
| Lumen / Nanite | 持续多年研发 | Clustered Forward+、CSM/IBL/SSAO；GI/虚拟几何列为研究 | 先保证稳定可玩；不承诺实时全局光照 |
| 图形 API / RHI | 多后端（DX12/Vulkan/Metal） | **唯一 Vulkan 1.3 后端（vulkanalia）**；Slang→SPIR-V | 单后端缩小矩阵，Windows/Linux 复用；代价是没有 DX12 兜底，驱动兼容性需矩阵测试 |
| Slate / UMG | 464 + 314 文件 | 运行时自研保留式 UI（`mir-ui`）；编辑器核心 `mir-editor-core` 与 UI 框架解耦（GPUI 候选 / egui 可选） | 运行时与工具分离、UI 可替换；代价是需维护 UI 适配层与视口合成方案 |
| `.uasset` / IoStore / Pak | 复杂二进制 | 自有 `.miasset` / `.mipak`，mmap + 压缩块 | 简单、可版本化、可 fuzz；代价是迁移需导入器 |
| UBT / UHT / AutomationTool | C# 工具链 | cargo + xtask + 过程宏 + `miriya-cli` | 与 Rust 生态一致；代价是构建缓存/增量需自建 |
| `.ini` 配置 | 大量层级配置 | TOML + 强类型 `Config` 结构 | 可校验、可补全；代价是文档需重写 |
| 平台抽象 | 多平台 RHI/Win32 等 | winit + 平台 trait；**当前只实现 Windows，Linux 接口保留** | 单平台交付、接口不锁死；代价是 Linux 后端需持续编译检查防锈化 |
| 单位/坐标系 | 厘米、Z-up、左手系、X 前 | **与 UE 对齐**，在物理/渲染/glTF 边界集中转换 | 便于复用美术习惯与教程；代价是边界转换必须集中且有测试 |

**核心依赖选型**（版本在 M0 统一钉在 `[workspace.dependencies]`，以实际兼容性为准）：

- 窗口/输入：`winit`、`gilrs`、`raw-window-handle`
- 图形：**`vulkanalia`**（唯一图形后端）、自研 Vulkan 子分配器、`slangc`（Slang→SPIR-V，离线预编译）、`spirv-reflect`、`bytemuck`
- 数学：`glam`
- ECS：`hecs`（内部实现，API 可换）
- 物理：**无内置求解器**；`mir-physics` 只定义接口与调度，实际后端为独立 crate/插件（用户自研）
- 音频：`cpal`、`symphonia`
- 图像/资产：`gltf`、`image`、`ktx2`、`zstd`、`lz4_flex`、`notify`
- 序列化/配置：`serde`、`toml`、`postcard`
- 反射/对象：`slotmap`、自研 `mir-reflect` 过程宏
- C# 宿主：`.NET 11`、`netcorehost` / `hostfxr-sys`（托管）、C# 绑定生成（`mir-bindgen`）
- 编辑器：`mir-editor-core`（无 UI 依赖）；UI 后端为可选项——`gpui`（候选）/ `egui`（备选），仅链接进编辑器二进制
- 异步/并行：`rayon`（CPU 作业）、`crossbeam-channel`（线程通信）
- 诊断：`tracing`（与 `mir-log` 桥接）、`tracy`（可选）、`criterion`、`cargo-fuzz`、`spirv-val`（CI）

### 4.2 Rust 语言特性使用策略

**原则**：同一语义下优先 Rust 惯用法，而不是复刻 C++/UE 的对象模型；只用稳定版特性，禁止为「炫技」
引入 nightly。具体取舍：

| Rust 特性 | 本项目中的用法 | 不采用的 C++/UE 习惯 |
|---|---|---|
| 所有权/借用/RAII | 资源、锁、句柄与 GPU 对象的生命周期；Drop 释放 | `UObject` GC、裸指针所有权、手工 `new/delete` |
| newtype | `Meters`/`Centimeters`、`Entity`/`AssetId`/`BodyId` 世代句柄 | `typedef` + 宏、同型交错传参 |
| `enum` + `match` | 状态机、事件、错误、后端选择、UI 命令 | 虚函数分派、`bool` 标志堆、`switch` |
| trait + 泛型（静态分发优先） | 物理/渲染/编辑器 UI 后端；策略可单态化 | 纯虚基类、运行期继承 |
| `dyn Trait`（边界使用） | 跨 crate 插件、运行时后端切换、反射访问 | 手工 vtable、对象切片 |
| `Result` + `thiserror` / `Option` | 可恢复错误、缺失资源、查询失败 | 异常控制流、`bool` + out 参数、`nullptr` |
| 迭代器/闭包 | 资产、渲染提取、ECS 遍历；零成本链式处理 | 下标循环 + 函数指针 |
| derive/属性宏 | `Reflect`、`Component`、`Config`、C# 绑定生成 | UHT 生成 `.generated.h` |
| 泛型 + `const` 泛型 | 数学、固定数组、管线布局、纹理格式 | 模板/宏拼接 |
| 生命周期 + `Cow`/`AsRef`/`Into` | 零拷贝资产路径、配置、跨边界的字符串视图 | `const&` + 拷贝、`char*` |
| feature flag + `#[cfg]` | Windows/Linux、UI 后端、格式与调试功能裁剪 | `#ifdef` 平台宏 |
| `rayon` + 通道 | CPU 作业池、资产导入、并行提取 | 自建线程池 + 全局锁 |
| 类型状态/私有构造 | 资源初始化状态、RenderPass 录制阶段 | `bInitialized` 标志位 |
| `#[non_exhaustive]` / sealed trait | 公共 API 演进与扩展点约束 | 头文件 ABI、友元/宏开关 |
| `unsafe` 局部化 | 仅 FFI/Vulkan/音频回调；每处 `// SAFETY:` | 全局裸指针/C 风格强转 |
| `#[must_use]` / Clippy | 返回值、错误与泄漏防护 | 注释约定 |

补充约束：

- **主循环不用 async runtime**：CPU 并行用 `rayon`/作业系统，I/O 用阻塞 + 线程池；网络（环 2）再评估 async。
- **不模拟 C++ 对象布局**：没有虚表兼容、没有 `UObject` 前置头、没有 `Get()`/`Set()` 成对的样板 API。
- **错误是值**：引擎/资产解析路径不使用 `panic!` 做控制流；只有测试、不可恢复的内部不变量才允许断言。
- **API 命名遵循 Rust**：`snake_case`、`try_`/`into_`/`as_` 前缀、`From`/`AsRef` 转换，不照搬 UE 缩写。

---

## 5. 架构设计

### 5.1 分层与依赖方向

```text
应用层   miriya-app · 游戏项目 · miriya-editor
门面     miriya-engine（re-export 运行时子系统）
工具层   miriya-cli · mir-build · mir-pak · mir-bindgen
编辑器   mir-editor-core · mir-editor-ui · mir-editor-gpui / mir-editor-egui（仅编辑器二进制）
运行时   mir-render · mir-scene · mir-physics（仅接口） · mir-animation · mir-audio · mir-ui · mir-gameplay · mir-script · mir-net
数据层   mir-asset · mir-serialize · mir-shader · mir-uasset
平台层   mir-window · mir-input · mir-platform
基础层   mir-core · mir-math · mir-log · mir-reflect · mir-tasks · mir-ecs · mir-object · mir-utils
```

**硬性规则**：

1. 底层 crate 不依赖上层；`mir-render` 不依赖 `mir-scene`（通过 `Extract` 帧数据解耦）。
2. **图形后端只有一个**：`mir-render` 通过 `vulkanalia` 独占 Vulkan；Vulkan 类型只出现在 `mir-render` 内部，
   禁止在运行时/编辑器公共 API 暴露；项目不依赖 wgpu、ash、DX12、OpenGL、Metal。
3. 平台层只做平台能力（窗口、设备、文件、计时），不承载玩法或托管运行时。
4. 每个子系统提供 headless/空后端（Null），保证 CI 与专用服务器可运行。
5. 子 crate 必须显式声明依赖，禁止通过 facade 的 re-export 形成隐式耦合。
6. **当前只实现 Windows**：平台能力以 trait 定义，Linux 提供占位实现并纳入编译检查；公共 API 不得出现
   Windows 专有句柄（`HWND`、`HINSTANCE` 等）。
7. C# 与引擎的边界只有**版本化 C ABI**：Rust 不直接依赖 .NET 内部结构，C# 不持有裸 Rust 指针。
8. **物理只留接口**：`mir-physics` 不包含求解器、刚体/碰撞算法或角色控制器；外部后端以独立 crate/插件注入，
   运行时 crate 不得依赖任何具体物理引擎。
9. **编辑器分层**：`mir-editor-core` 不依赖 UI 框架、窗口库或 Vulkan；UI 适配层（GPUI/egui/headless）
   只链接进 `miriya-editor`，游戏运行时与 facade 不得依赖编辑器或 UI 框架。
10. **Rust 惯用法优先**：公共 API 与内部设计使用所有权/借用、newtype、`enum`、trait/泛型、`Result`、
    迭代器等 Rust 特性；不模拟 C++ 继承/虚表/`UObject` 布局（见 §4.2）。

### 5.2 每帧流水线

```text
run():
  init platform → vulkan → dotnet host → assets → plugins
  loop:
    poll_events (winit)          # 窗口/输入/退出
    begin_frame                  # 时间、输入快照、资源上传
    fixed_update × N             # 引擎驱动固定步；具体求解由外部物理后端完成
    update                       # Rust 系统 + C# 脚本回调、动画采样、音频提交
    extract                      # ECS → 渲染帧数据（纯数据，可并行）
    render                       # RenderGraph：阴影→深度→不透明→透明→后处理→UI
    present
    end_frame                    # 清理按键沿、统计、帧限速
  shutdown（C# OnStop → 插件逆序）
```

- 默认**可变步长**（`dt` 上限 100 ms，防断点后物理爆炸）；提供固定步长模式（物理/网络/确定性测试）。
- 渲染与逻辑解耦：`extract` 只产出帧数据（相机、光源、实例、材质），渲染层不访问 ECS。
- 退出条件统一为 `ExitRequested` 资源（窗口关闭、应用请求、致命错误三路来源）。

### 5.3 线程模型

- **主线程**：winit 事件循环、帧调度、Vulkan 命令提交与 present（surface 与事件循环同线程）。
- **CPU 作业池**（rayon）：资产导入/解码、网格处理、动画采样、可见性粗筛；结果经帧同步回主线程。
- **音频线程**：cpal 回调，只读命令队列，不阻塞、不分配；**禁止在此调用 C#**。
- **C# 运行时**：脚本只在主线程的 `update` 阶段执行；.NET GC 线程由运行时管理，引擎不假设其时机。
- **物理线程（可选，v1.x）**：由外部物理后端决定是否使用独立线程；引擎默认在主线程按固定步调用后端，
  便于确定性与调试。
- 每帧的跨线程数据用双缓冲/句柄传递，禁止持锁做长任务；渲染/音频线程禁止进入托管代码。

### 5.4 插件与项目配置

- `Plugin` trait：`register(engine)`，向 `Schedule`/`RenderGraph`/资产类型注册系统与资源；卸载按逆序。
- 一等 crate 即插件（feature flag 可裁剪编译），第三方以独立 Rust crate 或 **C# 程序集**接入（不使用 WASM）。
- 项目清单 `Miriya.toml`：项目名、启动关卡、渲染配置（`backend = "vulkan"`）、物理后端、编辑器 UI 后端、
  音频配置、插件列表、C# 程序集（`[scripting]`）、资产目录。

### 5.5 Windows 当前实现 / Linux 接口保留

- 平台能力以 trait 定义：窗口、文件系统、计时、线程策略、音频设备、崩溃上报；实现按平台拆分，
  公共 API 只暴露跨平台抽象与 `raw-window-handle`。
- Windows 是唯一可发布实现；Linux 提供占位实现（返回 `Unsupported` 或明确错误）并参与
  `cargo check --target x86_64-unknown-linux-gnu` 编译检查，防止接口被 Windows 专有类型污染。
- Vulkan（`vulkanalia`）与 winit 本身跨平台；Linux 启用时只替换平台后端、路径策略与设备枚举，
  `mir-render`、`mir-script` 与玩法代码不改。
- Linux 的运行时验收、打包与支持承诺属于环 2（N6）；当前 CI 只保证「接口不腐烂」。

| 平台 | 当前状态 | 接口/实现 | 发布 |
|---|---|---|---|
| Windows x64 | 唯一实现 | 完整平台后端 + Vulkan 1.3 | ✅ 发布 |
| Linux x64 | 占位 | 平台 trait + `Unsupported` 实现 + CI 编译检查 | ⬜ 暂不发布（N6） |
| macOS / 移动端 / 主机 | 不做 | 不维护 | ❌ |

### 5.6 物理接口（不实现引擎）

- `mir-physics` 只定义：`PhysicsBackend` trait、`BodyId`/`ColliderId` 世代句柄、固定步驱动、
  查询/接触事件/调试绘制数据、可选的序列化挂钩。
- **不包含**：宽相/窄相、约束求解、刚体动力学、角色控制器、布料/破坏；这些全部由外部后端 crate 提供。
- 外部后端通过 `PhysicsPlugin` 注册；引擎内置 `NullPhysicsBackend` 供 CI/服务器；提供可复用的后端契约测试。
- 引擎负责坐标转换（UE Z-up ↔ 后端约定）、步进顺序、调试绘制收集；**不保证**外部后端的确定性，
  确定性由后端自行声明与测试。

### 5.7 编辑器与 UI 框架解耦

```text
miriya-editor（组合根，选择 UI 后端）
├── mir-editor-gpui / mir-editor-egui   # UI 适配层，只做事件/呈现/合成
├── mir-editor-ui                       # UI 宿主契约与视口协议
└── mir-editor-core                     # 文档/命令/Inspector/视口/PIE（UI 无关）
    └── miriya-engine                   # 运行时与渲染（Vulkan/vulkanalia）
```

- `mir-editor-core`：UI 无关的纯状态机——文档/关卡、选择、命令与撤销、Inspector 模型、视口相机/Gizmo 数学、
  PIE 会话、资产浏览器模型；只依赖引擎运行时与数学/反射，不依赖 winit/Vulkan/egui/GPUI。
- `mir-editor-ui`：UI 宿主契约（事件翻译、命令/面板映射、视口目标）；`mir-editor-gpui` 与
  `mir-editor-egui` 为可选实现；`miriya-editor` 是组合根，按 feature 选择后端。
- 视口渲染仍由 `mir-render`（`vulkanalia`）执行；编辑器 UI 只消费渲染结果（引擎自有 surface 或离屏纹理合成）。
- 事件循环由 UI 宿主实现（GPUI 自带；egui 由 winit 等适配层驱动）；PIE 是核心驱动的子会话，
  编辑器 UI 依赖不会进入游戏运行时。
- 若 GPUI 自带 GPU 渲染器，它只负责编辑器 chrome，不改变游戏/视口的 Vulkan 后端；若无法共享纹理或事件循环，
  优先实现离屏合成/子表面适配；若要求编辑器 chrome 也必须走 `vulkanalia`，则实现 GPUI-on-Vulkan 适配或改用
  自绘 UI，仍不可行时回退 egui（ADR-0009 记录结论）。
- M12 先做 GPUI 与 egui 的 spike，再由 ADR-0009 选定默认后端；无论选哪个，`mir-editor-core` 必须通过 headless 测试。

---

## 6. 仓库结构与 crate 规划

### 6.1 当前结构（M0 骨架已落地）

workspace 已按目标结构拍平，`cargo metadata` 验证为 **41 个成员**（37 个 crates + 4 个 tools）；
其中 13 个已有实现（门面、示例、11 个引擎子 crate，含 `mir-physics` 与 `mir-editor-core` 的契约骨架），
新增 `mir-vulkan-probe` PoC 工具，其余 27 个为仅含模块文档的结构占位。

```text
Miriya/
├── Cargo.toml                  # members = ["crates/*", "tools/*"]
├── rust-toolchain.toml         # stable
├── .editorconfig
├── README.md
├── crates/
│   ├── miriya-engine/          # 门面（现有）
│   ├── miriya-app/             # 示例（现有，bin = miriya）
│   ├── miriya-editor/          # 编辑器组合根（占位，M12）
│   ├── mir-{core,math,ecs,input,asset,render,window,scene,log}/   # 9 个现有引擎子 crate
│   ├── mir-editor-{core,ui,gpui,egui}/                            # 编辑器分层（占位）
│   └── mir-{tasks,utils,reflect,object,serialize,shader,platform,
│            physics,audio,animation,ui,gameplay,net,cinematic,nav,
│            vfx,pak,image,uasset,script,terrain}/                 # 其余规划 crate（占位）
├── tools/                      # miriya-cli、mir-build、mir-bindgen（占位）+ mir-vulkan-probe（PoC）
├── managed/                    # Miriya.Runtime / Miriya.Scripting（csproj 空壳）
├── assets/ config/ shaders/ tests/ docs/{adr,book,rfcs}/          # 占位目录
└── target/                     # 构建产物（git 忽略）
```

### 6.2 目标结构（M0 骨架已创建，内容按里程碑填充）

M0 已完成拍平、workspace 成员收敛与空壳创建；以下为完整目标结构：

```text
Miriya/
├── Cargo.toml                  # members = ["crates/*", "tools/*"]
├── rust-toolchain.toml         # 固定工具链（见 §11.1）
├── .editorconfig               # 编辑器格式约定
├── README.md                   # 本文件
├── LICENSE-MIT / LICENSE-APACHE
├── crates/
│   ├── miriya-engine/          # 门面（原 crates/engine）
│   ├── miriya-app/             # 示例/模板应用
│   ├── miriya-editor/          # 编辑器组合根（M12）
│   ├── mir-editor-core/        # UI 无关编辑器模型/命令/视口/PIE
│   ├── mir-editor-ui/          # UI 宿主契约与视口协议
│   ├── mir-editor-gpui/        # GPUI 适配后端（候选）
│   ├── mir-editor-egui/        # egui 适配后端（备选）
│   ├── mir-core/               # 循环、配置、插件、时间
│   ├── mir-tasks/              # CPU 作业池、帧同步
│   ├── mir-utils/              # 基础工具（路径、哈希、容器适配）
│   ├── mir-math/               # 数学与坐标约定
│   ├── mir-log/                # 日志（现有，已可用）
│   ├── mir-reflect/            # 反射 derive 与注册表
│   ├── mir-object/             # 对象/句柄/生命周期
│   ├── mir-ecs/                # hecs 封装 + 调度
│   ├── mir-scene/              # Level/Actor/Component
│   ├── mir-asset/              # 资产服务器/注册表/导入器
│   ├── mir-serialize/          # 版本化序列化
│   ├── mir-shader/             # Slang→SPIR-V、反射、缓存、热重载
│   ├── mir-render/             # Vulkan（vulkanalia）与 RenderGraph（M2）
│   ├── mir-window/             # winit 窗口
│   ├── mir-input/              # 输入动作映射
│   ├── mir-platform/           # 文件/系统/计时等平台服务
│   ├── mir-physics/            # 仅接口/调度/查询/调试（无求解器）
│   ├── mir-audio/              # 混音器/空间化
│   ├── mir-animation/          # 蒙皮/动画图
│   ├── mir-ui/                 # 运行时 UI
│   ├── mir-gameplay/           # 游戏框架
│   ├── mir-net/                # 复制（v1.x）
│   ├── mir-cinematic/          # 序列/过场（v1.x）
│   ├── mir-nav/                # 导航/AI（v1.x）
│   ├── mir-vfx/                # 粒子/VFX（研究）
│   ├── mir-pak/                # .mipak 打包
│   ├── mir-image/              # PNG/KTX2 解码
│   ├── mir-uasset/             # 可选：UE 只读导入（实验）
│   ├── mir-script/             # C# 托管：hostfxr、绑定、可卸载 ALC 热重载
│   └── mir-terrain/            # 地形/植被（v1.x）
├── managed/
│   ├── Miriya.Runtime/         # C# 引擎封装与生成的绑定（.NET 11）
│   └── Miriya.Scripting/       # 用户脚本基类、模板与示例
├── tools/
│   ├── miriya-cli/             # new/run/build/cook/pack（M13）
│   ├── mir-build/              # 构建与烘焙库
│   └── mir-bindgen/            # 从 mir-reflect 生成 C# 绑定
├── assets/                     # 示例资产（源）
├── config/                     # TOML 配置
├── docs/
│   ├── adr/                    # 架构决策记录 NNNN-*.md
│   ├── book/                   # mdbook 手册
│   └── rfcs/                   # 大特性提案
├── shaders/                    # Slang 源与公共 module
└── tests/                      # 跨 crate 集成/golden/基准
```

### 6.3 当前 crate 状态

| crate | 现状 | 成熟度 | 下一步（里程碑） |
|---|---|---|---|
| `mir-core` | `EngineConfig`（含 `validation`）/主循环、帧率限制、日志初始化 | 🟡 原型 | 拆 `EngineLoop`/`FramePipeline`、插件与退出资源（M2–M3） |
| `mir-math` | glam newtype：`Vec2/3/4`、`Mat3/4`、`Quat`，含测试 | 🟡 原型 | `Transform` 收敛到本 crate、补 `Aabb`/`Frustum`/坐标系转换（M0） |
| `mir-ecs` | `HashMap<TypeId, HashMap<Entity, Box<dyn Any>>>`，递增 `Entity` | 🟡 原型 | 换 hecs、查询/调度/变更检测（M3） |
| `mir-input` | winit 事件驱动的键鼠状态、按键沿、鼠标增量与失焦清理 | 🟡 原型 | Action 映射、键位重绑、手柄（M7） |
| `mir-render` | Vulkan 1.3 完整绘制链路：交换链/深度/网格上传/UBO/push constants/10 实例绘制、校验层与日志桥接、2 帧在飞行、RenderGraph v0.5（自动 barrier/别名/GPU 时间戳/dump）、单 `.spv` 多入口、管线缓存持久化 | 🟡 原型 | 显存子分配器、材质/PBR（M2 批次 E/F/M5） |
| `mir-window` | winit 0.30 窗口/事件循环、raw-window-handle、光标捕获与失焦自动释放 | 🟡 原型 | DPI 缩放、挂起/恢复处理（M2 剩余） |
| `mir-asset` | `Handle`/`AssetServer` 同步缓存雏形，失败即 panic | 🟡 原型 | 异步加载、注册表、导入器、错误化（M4） |
| `mir-scene` | `Scene`、`Transform`（含旋转/TRS）、UI 无关的自由相机与控制器 | 🟡 原型 | Level/Actor/层级/组件（M3） |
| `mir-log` | 全局 logger、五级宏、控制台输出、含单元测试，纯 std | ✅ 可用 | `tracing` 桥接、文件/异步 writer（M1） |
| `mir-physics` | `PhysicsBackend` 契约、世代句柄、`FixedStep`、查询/接触事件/调试数据、`NullPhysicsBackend`、可复用契约测试；无求解器 | 🟡 契约 | 引擎侧胶水与外部后端接入（M6） |
| `mir-editor-core` | UI 无关的文档/选择/类型化命令与撤销栈/Inspector 字段描述/视口数学/PIE 状态机，headless 测试 | 🟡 契约 | 完整编辑器核心与 UI 后端 spike（M12） |

> 现状细节与已知问题见 §14。除上表 11 个已有内容的子 crate 外，其余 crate 目前均为空壳（仅含模块文档）；
> 仓库不包含任何物理求解器实现。

---

## 7. UE 模块映射

> 上游模块数量众多（Runtime 194 个），此处只列**主线模块**。其余模块（Online、Friends、Media、
> WebBrowser、AR 等）默认不做，除非环 2/3 明确纳入。映射只表示**职责归属**，不是 1:1 文件/类迁移；
> Miriya 的公共 API 按 Rust 惯用法重新设计（§1.3 / §4.2）。

| UE 模块（规模） | Miriya 对应 | 优先级 | 里程碑 | 语义对齐方式（非 1:1） |
|---|---|---|---|---|
| `Core`（2,105 文件） | `mir-core` / `mir-tasks` / `mir-utils` | P0 | M0–M1 | 行为对齐（循环、作业、容器、平台抽象） |
| `MathCore` / `Math` | `mir-math` | P0 | M0 | 行为对齐 + UE 坐标约定 |
| `CoreUObject`（956） | `mir-reflect` / `mir-object` | P0 | M0、M3 | 近似替代（derive 代替 UHT） |
| `Engine`（4,007，Actor/Component/World） | `mir-scene` / `mir-gameplay` | P0 | M3、M7 | 行为对齐（Actor 模型） |
| `MassEntity` / ECS 相关 | `mir-ecs` | P0 | M3 | 近似替代（hecs） |
| `RHI` + `RenderCore` + `Renderer` | `mir-render` / `mir-shader` | P0 | M2 | 替代（唯一 Vulkan 后端，`vulkanalia`） |
| `ApplicationCore` / `Launch` / `Windows` | `mir-platform` / `mir-window` | P0 | M1 | 近似替代（winit） |
| `InputCore` / Enhanced Input | `mir-input` | P0 | M1、M7 | 行为对齐（Action/MappingContext） |
| `AssetRegistry` / `Serialization` / `PakFile` | `mir-asset` / `mir-serialize` / `mir-pak` | P0 | M4、M13 | 自有格式 + 只读导入 |
| `ImageCore` / `ImageWrapper` | `mir-image` | P1 | M4 | 近似替代（PNG/KTX2） |
| `BlueprintRuntime` / `BlueprintEditor` | `mir-script`（C#） | P0 | M8 | 替代：**不使用蓝图**，C# 脚本承担玩法 |
| `PhysicsCore` / Chaos | `mir-physics`（仅接口） | P0 | M6 | **不实现**；外部自研后端注入，接口契约见 §9.9 |
| `AudioMixer` / 音频设备 | `mir-audio` | P0 | M9 | 近似替代（cpal + 自研混音） |
| `AnimationCore` / `AnimGraphRuntime` | `mir-animation` | P0 | M10 | 行为对齐（图/混合/根运动） |
| `Slate` / `SlateCore` / `UMG` | `mir-ui` + `mir-editor-core` + UI 后端 | P1 | M11–M12 | 替代（不做 Slate；编辑器支持 GPUI/egui 适配） |
| `GameplayTags` / `GameplayTasks` / `DeveloperSettings` | `mir-gameplay` / `mir-core` | P1 | M7 | 行为对齐（精简） |
| `Projects` / `BuildSettings` | `mir-core`（配置） + `Miriya.toml` | P0 | M1 | 替代（TOML） |
| `Json` / `XmlParser` / `Cbor` | serde 生态 | P0 | M4 | 替代 |
| `MovieScene` / Sequencer | `mir-cinematic`（v1.x） | P2 | 环 2 | 延后 |
| `Net` / `Networking` / 复制 | `mir-net` | P2 | 环 2 | 延后（固定步长快照） |
| `Landscape` / `Foliage` | `mir-terrain` | P2 | 环 2 | 延后 |
| `NavigationSystem` / AI | `mir-nav` | P2 | 环 2 | 延后 |
| Niagara（插件） | `mir-vfx` | P2 | 环 3 | 研究 |
| MetaSounds | `mir-audio` 图子系统 | P2 | 环 2 | 延后 |
| `UnrealBuildTool` / `UnrealHeaderTool` | cargo + xtask + `mir-reflect` 宏 | P0 | M0 | 替代 |
| `UnrealPak` / IoStore | `mir-pak` / `miriya-cli` | P0 | M13 | 替代 |
| `UnrealInsights` / `TraceLog` | `tracing` + tracy 可选 | P1 | M1 | 替代 |
| `UnrealEd`（1,541） | `mir-editor-core` / `miriya-editor` | P1 | M12 | 精简替代（不做全功能；UI 框架可插拔） |
| `AutomationTool` / `AutomationTest` | cargo test + 自建 golden | P1 | M1 | 替代 |
| `ThirdParty`（595.5 MB） | 按需引入 Rust crate | P0 | 持续 | 不整体移植 |

**不映射**：`Online`、`FriendsAndChat`、`Media`、`WebBrowser`、`CEF3Utils`、`AugmentedReality`、
`OpenColorIOWrapper`、`CUDA`、`D3D12RHI`/`VulkanRHI` 的私有路径、`VerseCompiler`、`Solaris`、
`VirtualProduction` 等。它们要么由 Vulkan/第三方 crate 覆盖，要么不属于桌面游戏主线。

---

## 8. 里程碑与路线图

### 8.1 估算前提

- 团队假设：**3–4 名 Rust 工程师（至少 1 名熟悉 .NET/C#）+ 1 名技术美术/QA**；当前只发布 Windows，
  Linux 仅保留接口并做编译检查。外部物理后端与编辑器 UI 选型不计入本估算。
- 人周为**净开发估算**，不含需求返工、内容制作与发布活动；计划按 **1.6×** 缓冲做排期。
- 估算总计约 **130 人周**；含缓冲约 **208 人周** → 4 人团队约 **12–17 个月**，
  单人开发者约 **3–4 年** 达到 v1.0。任何「三个月重写 UE」的说法都不现实。

### 8.2 里程碑

| ID | 里程碑 | 主要交付 | 退出标准（Gate） | 人周 |
|---|---|---|---|---:|
| **M0** | 工程基线收敛 | workspace 收敛、LICENSE、CI、rust-toolchain、ADR、拍平/成员补齐、facade 导出修正、`vulkanalia`/`.NET` 依赖 spike | `cargo fmt/clippy/test --workspace` 全绿；Windows 测试 + Linux 编译检查 | 2 |
| **M1** | 平台与循环（Windows） | winit 窗口/事件、Linux 平台 trait 占位、退出链路、输入快照、日志桥接、帧统计、`FramePipeline` | 打开窗口、按键/缩放、关闭可退出、headless 1000 帧测试、Linux `cargo check` 通过 | 4 |
| **M2** | 渲染 v0（Vulkan） | `vulkanalia` 实例/设备/交换链、Slang→SPIR-V、深度、相机、网格/实例、基础 PBR、RenderGraph（含单 `.spv` 多入口整合）、管线缓存 | v0.2「旋转立方体 + 自由相机」1080p 稳定；Vulkan 校验层无错误 | 12 |
| **M3** | 场景与 ECS | hecs 封装、Actor/Component、层级变换、序列化往返、场景加载/保存 | 10k 实体变换基准达标；关卡存取 golden 一致 | 8 |
| **M4** | 资产管线 | 异步加载、注册表、glTF/纹理/音频导入、依赖图、热重载、`.miasset` | 1 GB 场景不卡主线程；热重载 200 ms 内生效 | 10 |
| **M5** | 材质与光照 | Slang module、材质参数/纹理、IBL、CSM、SSAO、Bloom/色调映射 | v0.3「PBR 场景」1080p 60fps（中端独显） | 10 |
| **M6** | 物理接口（不含实现） | `PhysicsBackend` trait、世代句柄、固定步驱动、查询/事件/调试绘制、Null 后端、可复用契约测试 | mock 后端通过契约测试；外部后端可按文档接入；仓库内无求解器 | 6 |
| **M7** | 游戏框架与输入 | GameMode/Pawn/Controller、输入 Action 映射、HUD 接口、存档 | 可玩 FPS 闭环的 Rust 侧骨架（移动/射击/死亡/重开） | 6 |
| **M8** | C# 脚本运行时 | .NET 11 hostfxr 托管、版本化 C ABI、绑定生成、脚本生命周期、可卸载 ALC 热重载、C# 模板 | **用 C# 实现玩家移动/生成 Actor**；热重载 ≤ 1 s；无任何蓝图依赖 | 8 |
| **M9** | 音频 | cpal 输出、混音器、解码、空间化、总线/效果 | 无爆音、设备切换稳定、offline 渲染测试 | 6 |
| **M10** | 动画 | GPU 蒙皮、片段采样、混合树/状态机、根运动、IK 基础 | 50 角色 × 30 骨骼 60fps | 12 |
| **M11** | 运行时 UI | 控件树、布局、输入路由、SDF 字体、UI pass、主题 | HUD/菜单/设置/暂停；UI ≤ 1 ms | 8 |
| **M12** | 编辑器 | `mir-editor-core`（文档/命令/撤销/Inspector/视口/Gizmo/PIE）+ UI 后端 spike（GPUI 候选、egui 备选）+ 视口合成 + C# 脚本构建 | 核心 headless 测试全绿；选定 UI 后端可编辑/保存/PIE；视口由 vulkanalia 渲染 | 28 |
| **M13** | 打包与工具链（Windows） | `miriya new/run/build/cook/pack`、`.mipak`、自包含 .NET 运行时、shader 预编译 | v0.6「独立 exe + 资产包」；干净机器可运行；Linux 接口 check 通过 | 10 |

### 8.3 垂直切片（每个都是可演示版本）

| 版本 | 演示内容 | 覆盖里程碑 |
|---|---|---|
| **v0.1 Hello GPU** | 窗口 + 清屏 + 三角形 | M0–M2 |
| **v0.2 Cube** | 透视相机、深度、网格、Transform、自由飞行 | M2 |
| **v0.3 PBR Scene** | glTF 导入、IBL、CSM、色调映射 | M2–M5 |
| **v0.4 Playable** | C# 脚本驱动的角色移动/跳跃/射击、物理（**依赖外部后端**）、HUD、音效 | M6–M9 |
| **v0.5 Editor Lite** | 打开/保存关卡、拾取、Gizmo、Inspector、PIE、C# 脚本热重载 | M10–M12 |
| **v0.6 Package** | Cook + 打包 + 独立运行（Windows） | M13 |
| **v1.0 Parity** | 复刻第一/第三人称模板工作流、Windows + Vulkan、C# 玩法、文档齐全 | 全部 |

> 注：v0.4 的角色移动/碰撞需要一个外部物理后端；若用户后端未就绪，仅用 mock 后端验证接口契约，
> 不把玩法行为计入验收。物理后端开发不属于 Miriya 工作量（见 §8.1）。

### 8.4 环 2 与环 3（时间待 v1.0 后重估）

| 编号 | 方向 | 依赖 | 粗估 |
|---|---|---|---|
| N1 | 网络复制（服务器权威、快照插值） | M7、固定步长 | 12–16 人周 |
| N2 | World Partition 简化版（关卡流式） | M3、M4 | 10–14 人周 |
| N3 | 高级渲染（SSGI/DDGI、虚拟几何研究） | M2、M5 | 研究型，20+ 人周 |
| N4 | 动画进阶（重定向、压缩、IK 完善） | M10 | 8–12 人周 |
| N5 | 富运行时 UI（数据绑定、复杂控件） | M11 | 10–14 人周 |
| N6 | 启用 Linux（复用已保留的平台接口） | M1、M13 | 8–12 人周 |

> 外部物理后端与编辑器 UI 框架不在 N 列：前者由用户自研（Miriya 只提供契约），后者在 M12 的 ADR-0009 中定选型。

### 8.5 Definition of Done（所有里程碑通用）

- 单元/集成测试与文档齐全；`cargo test --workspace` 与 Clippy 全绿。
- 公共 API 通过 Rust 惯用法评审（§4.2）：所有权/借用、newtype、`enum`、`Result`；无 C++ 式继承/裸指针样板。
- C# 侧 `dotnet build/test` 通过；Vulkan 校验层在 debug/CI 无错误。
- 物理接口契约测试通过（mock 后端）；编辑器核心 headless 测试通过（不依赖 UI 框架）。
- 至少一个可运行 demo 或可复现命令；性能数据记入基准报告。
- 新增公共 API 有 rustdoc 示例；架构性变更有 ADR。
- 不引入已知崩溃/内存泄漏（`cargo miri` 或 sanitizer 抽查关键 crate）。
- 里程碑验收由至少一名非作者评审。

---

## 9. 子系统设计要点

### 9.1 `mir-core`：循环、调度与生命周期

- 保留 `AppState` 作为应用入口，但 `Engine::run` 拆为 `EngineLoop` + `FramePipeline`，便于 headless/编辑器复用。
- `Schedule` 阶段：`PreUpdate` / `FixedUpdate` / `Update` / `PostUpdate` / `Extract` / `Render` / `Shutdown`。
- `Time` 可注入虚拟时钟（测试、录制、离线渲染）；`dt` 上限 100 ms；帧限速与 vsync 二选一（避免双重限制）。
- 统一 `ExitRequested` 资源；插件注册/卸载逆序；致命错误走 `Result` 上报而非 `panic!`。
- 验收：无窗口 headless 跑 1,000 帧并干净退出；固定步长下的确定性测试。

### 9.2 `mir-math`：数学与坐标约定

- **全局约定**：厘米、Z-up、左手系、X 前、Y 右（与 UE 一致），在文档与类型注释中固定。
- 继续使用 glam 的 newtype 封装策略（`#[repr(transparent)]` + `Deref`），补 `Transform`、`Aabb`、`Frustum`、
  `Ray`、`Plane`、`Color` 与 `interop`（Vulkan NDC、外部物理后端坐标约定、glTF Y-up）。
  `Transform` 是纯数学类型，归 `mir-math` 所有；现有 `mir-scene::Transform` 在 M3 迁移并重导出，保持应用层导入路径稳定。
- 投影矩阵在**相机边界**转换到 Vulkan 约定（深度 0..1、NDC Y 向下）；物理/导入转换只允许在 `interop` 模块出现。
- 测试覆盖手性、投影、裁剪、四元数插值；`bytemuck::Pod` 供 GPU 零拷贝（后续）。
- 禁止在业务代码里手写坐标系翻转；发现即视为评审阻塞项。

### 9.3 `mir-reflect` / `mir-object`：反射与对象模型

- `#[derive(Reflect)]` 生成 `TypeInfo`（名称、字段、属性描述器、序列化/反序列化函数），注册进 `TypeRegistry`。
- **Rust 优先**：对象由句柄与所有权管理，不建立 `UObject` 式继承树；状态/事件用 `enum`，避免 `dyn Any` 泛滥。
- 对象用 `slotmap` 世代句柄 + `WeakHandle`；无 tracing GC，显式 `destroy`；环引用用弱引用或资产 ID 打破。
- 编辑器 Inspector、资产依赖、存档共用一份反射数据；字段可标注 `#[reflect(skip)]`、`#[reflect(range)]`。
- 与 UHT 的差异：类型在编译期确定，不支持运行时代码生成；这正是安全性与工具链简单性的来源。
- 验收：嵌套结构/枚举/资源引用可二进制 + 文本往返；Inspector 自动生成控件。

### 9.4 `mir-ecs`：数据层

- 内部采用 `hecs`（成熟、查询快、维护成本低），对外封装 `World`/`Query`/`System` 稳定的 API。
- 资源（`Resource`）与实体组件分离；`Schedule` 决定系统顺序；变更检测用版本号（Added/Changed）。
- Actor 元数据（名称、层级、反射类型）作为组件；纯运行数据（Transform、Renderable、RigidBody）走 ECS。
- 预留 archetype/并行查询扩展；不追求自研 ECS 的「炫技」特性。
- 验收：10k 实体 × 3 组件单线程变换更新 < 1 ms（记录基准机）。

### 9.5 `mir-scene`：关卡与 Actor

- `Level` = ECS World + 元数据 + 单元划分；`Actor` = 实体句柄 + 组件列表 + 层级 + Tick 策略。
- **Actor 是外观而非基类**：不模拟 UE 的继承层级；行为通过组件与 trait 组合，类型用 `enum`/泛型表达。
- `ActorComponent` 生命周期（BeginPlay/EndPlay/Serialize）显式定义；Tick 可按距离/可见性关闭。
- 引用用资产 ID（稳定），不用路径；保存/加载走反射序列化；PIE 通过克隆 World，不污染编辑器状态。
- 关卡流式（World Partition 简化）列为 N2；先实现单关卡 + 子场景挂载。
- 验收：保存→加载→再保存的 golden 一致；Actor 重命名/删除不破坏引用。

### 9.6 `mir-render` / `mir-shader`：Vulkan 渲染（唯一后端）

- **后端**：Vulkan 1.3，通过 **`vulkanalia`**。`mir-render` 内部负责实例、物理设备选择、队列、交换链、
  描述符池/布局、管线、显存、命令缓冲与同步；公共 API 不出现 Vulkan 类型。
- **帧数据边界（ADR-0012）**：应用层提取 `CameraFrame` / `InstanceData` 后提交，`mir-render` 不依赖场景或 ECS；
  相机放 UBO、逐实例数据放 push constants。v0.2 仍是单帧在飞行，多帧/子分配器为 M2 剩余。
- **特性**：动态渲染（dynamic rendering）、synchronization2、timeline semaphore、descriptor indexing；
  调试构建启用 Khronos validation layer；发布版持久化 pipeline cache 降低首帧卡顿。
- **帧结构**：2–3 帧在飞行、环形暂存缓冲、**自研显存子分配器**（块/池/延迟回收，基于 `vulkanalia` 的显存类型，
  不引入 ash）；交换链重建/设备丢失有恢复路径。
- **管线**：Clustered Forward+：CSM 阴影 → 深度预通道 → 不透明（PBR + IBL）→ 透明 → SSAO/Bloom/色调映射 → UI。
- **RenderGraph**：pass 声明读写、自动 barrier/layout 转换、临时纹理别名、GPU 时间戳；每帧可 dump 依赖图。
- **着色器**：Slang（公共 module + `import`）经 `slangc -fvk-use-entrypoint-name` 编译为单个 SPIR-V，
  保留 `vertexMain`/`fragmentMain` 入口名，Rust 侧单 ShaderModule 按入口名绑定；`spirv-reflect` 生成绑定；
  `spirv-val` 与校验层把关；离线缓存以管线哈希为键；开发模式热重载；材质参数走 UBO/SSBO。
- **平台**：Vulkan 为 Windows/Linux 通用 API；启用 Linux 时只需替换 surface/平台后端，渲染代码不变。
- **运行前提**：Windows 10/11 x64 + 支持 Vulkan 1.3 的显卡驱动；启动时检测 API/特性并给出可读错误，
  不提供 DX12 兜底。
- 目标：1080p 60fps（中端独显）、1M 三角形、GPU 帧时间 ≤ 16.6 ms；着色器编译不阻塞主循环。
- 约束：**不引入 wgpu/ash/DX12/OpenGL/Metal**；不做 Lumen/Nanite 等价，GI/虚拟几何在 N3 研究。

### 9.7 `mir-asset`：资产服务器

- `AssetId`（UUIDv7）+ `AssetRegistry`（路径/类型/依赖，不加载内容）；`Handle<T>` 引用计数。
- 异步加载：作业池解码，主线程只做 GPU 上传与依赖解析；句柄查询不阻塞。
- `Importer` trait：glTF（首选，含 Draco 可选）、PNG/KTX2、WAV/OGG/FLAC；FBX 后置（评估 `ufbx`）。
- 热重载：`notify` 监听源资产，构建哈希比对后重建；编辑器与运行时行为一致。
- 错误策略：解析错误返回 `AssetError` 并记录资产 ID，不 panic；损坏资产隔离，不拖垮进程。
- 验收：冷启动 1 GB 资产不阻塞主线程超过 1 帧；热重载 ≤ 200 ms。

### 9.8 `mir-input`：输入动作

- 事件（winit/gilrs）→ 设备状态 → `InputMappingContext` → `InputAction`（数字/轴/双轴）。
- 修饰器：死区、取反、缩放、连击/长按、组合键；UI 捕获优先级；暂停/失焦自动释放。
- 键位重绑与持久化（TOML），手柄热插拔；触摸后置。
- 验收：重绑、手柄插拔、暂停不穿透、编辑器内不抢运行时输入。

### 9.9 `mir-physics`：只提供接口，不实现物理引擎

- **范围**：不实现 Chaos，也不内置 Rapier/PhysX 等求解器；本 crate 只定义后端契约与引擎侧胶水。
- **接口**：`PhysicsBackend`（世界创建/销毁、`step(FixedStep)`、刚体/碰撞体添加删除、变换读写、
  raycast/sweep/overlap、接触事件、debug draw、可选序列化）；`BodyId`/`ColliderId` 为世代句柄。
- **引擎职责**：固定步累加与调用顺序、句柄表、查询转发、Z-up ↔ 后端坐标转换、调试绘制收集；
  **不包含**宽相/窄相、约束求解、角色控制器、布料/破坏。
- **接入**：外部后端以 `PhysicsPlugin` 注册（独立 crate，首选 Rust trait）；非 Rust 后端可自行加可选
  C ABI 适配层，契约保持不变。内置 `NullPhysicsBackend` 与可复用的 `physics-contract` 测试套件，
  后端作者可直接跑契约测试。
- **确定性**：由后端声明并测试；引擎只保证固定步与调用顺序一致，不以引擎名义承诺物理确定性。
- 验收：mock 后端通过契约测试；外部后端接入示例可运行；引擎侧调度/查询开销达标（不含后端求解时间）。

### 9.10 `mir-audio`：音频

- cpal 输出线程 + 自研混音器（总线/组/效果）；symphonia 解码；播放/暂停/循环/淡入淡出。
- 空间化：Listener、距离衰减、空气吸收；遮挡/混响 v1.x。
- 无设备模式（Null）用于 CI 与服务器；离线渲染导出 WAV 供测试比对。
- MetaSound 式音频图列为 N 系列，不阻塞 v1.0。
- 验收：设备切换/暂停/恢复无爆音；CI 可比对离线输出。

### 9.11 `mir-animation`：动画

- 骨架/蒙皮：glTF 导入、GPU 蒙皮（compute 优先，顶点着色器兜底）；法线/切线空间正确。
- `AnimationClip` 采样、混合树（1D/2D）、状态机（状态/转移/条件）、根运动提取、事件标记。
- 动画图先用 Rust/TOML 定义（可测试），编辑器面板后置；IK：TwoBone + FABRIK。
- 验收：50 角色 × 30 骨骼 1080p 60fps；根运动与胶囊体同步无漂移。

### 9.12 `mir-gameplay`：游戏框架

- `GameMode`/`GameState`/`PlayerController`/`Pawn`/`PlayerState` 对应物；生成、占有、重生、计分。
- 输入路由按控制器/Pawn；通用流程留在 Rust，**玩法规则由 C# 脚本实现**（通过 `mir-script` 暴露的类型与事件）。
- 保持薄层：不把具体玩法写进引擎，模板项目演示 C# 侧最佳实践。
- 验收：在 C# 中完成「出生 → 移动/射击 → 死亡 → 重开」完整闭环。

### 9.13 `mir-script`：C# 游戏脚本

- **目标**：C# 是唯一的玩法脚本语言；**不使用蓝图、节点图或 WASM 脚本**。
- **托管**：.NET 11 通过 `hostfxr`（`netcorehost`）在进程内加载；用户程序集由 `miriya build`
  调用标准 `dotnet build` 编译；`Miriya.Runtime` 提供引擎封装与生成的绑定。
- **边界**：版本化 C ABI（表驱动函数指针 + 不透明 64 位句柄）；C# 侧生成 `SafeHandle`，引擎校验句柄世代；
  禁止裸指针跨边界；字符串统一 UTF-8 + 显式所有权。
- **异常**：托管异常必须在 C ABI 边界转换为引擎错误码并附 C# 栈，禁止跨越 `extern "C"` 展开/传播。
- **序列化**：场景只保存 C# 类型名/稳定路径与参数，不序列化托管对象图；加载时由脚本重建实例。
- **编辑体验**：脚本项目是标准 `.csproj`，可在 Visual Studio / Rider / VS Code 中编辑与调试；
  Miriya 编辑器负责构建、加载与热重载，不做 C# 代码编辑器。
- **生命周期**：`OnStart/OnUpdate/OnStop` 与 Actor/组件回调在主线程执行；退出时先停脚本，再卸载插件。
- **热重载（开发）**：文件监听 + `dotnet build`；用户程序集放入可卸载 `AssemblyLoadContext`，
  重载后按稳定对象 ID 重新绑定；状态迁移尽力而为，失败则回退重启 PIE。
- **发布**：随包分发 .NET 11 自包含运行时（以体积换免安装）；Native AOT 列为后续可选优化（不支持热重载）。
- **性能**：不为每个实体做 P/Invoke；批量 `Span<T>`/native buffer；性能敏感路径留在 Rust；基准跟踪脚本开销。
- **安全**：用户程序集是受信代码（非沙箱）；禁止在渲染/音频线程调用托管代码。
- 验收：C# 脚本驱动玩家移动、生成/销毁 Actor、订阅输入；热重载 ≤ 1 s；打包后无需安装 SDK 即可运行。

### 9.14 `mir-ui`：运行时 UI

- 保留式控件树（Panel/Text/Button/Image/Scroll）+ 简化 Flex/Grid 布局 + 输入路由 + 焦点/导航。
- 渲染：独立 UI pass，SDF 字体图集，9-slice；主题与 DPI 缩放。
- 编辑器 UI 与运行时 UI 完全解耦；编辑器后端可插拔（GPUI/egui），见 §9.15。
- 验收：HUD、菜单、设置、暂停；1080p UI ≤ 1 ms；手柄可导航。

### 9.15 `mir-editor-core` / `mir-editor-ui` / `miriya-editor`：编辑器

- **分层**：`mir-editor-core` 是与 UI 框架无关的状态机——文档/关卡、选择、命令栈、Inspector 模型、
  视口相机/Gizmo 数学、拾取、PIE 会话、资产浏览器模型；不依赖 UI 框架、窗口库或 Vulkan。
- **Rust 优先**：编辑命令/选择集用类型化 `enum` 与结构体表达，不复制 UE 的 `UObject` 命令对象/继承体系。
- **UI 契约**：`mir-editor-ui` 定义 UI 宿主接口、事件映射与视口目标；`mir-editor-gpui`（候选）、
  `mir-editor-egui`（备选）实现该契约；`miriya-editor` 是组合根，按 feature flag 选择后端。
- **视口**：`mir-render` 渲染到引擎自有 surface 或离屏纹理，UI 后端负责合成/呈现；编辑器核心不接触 GPU。
- **事件循环**：由 UI 宿主实现（GPUI 自带；egui 由 winit 适配层驱动）；PIE 是核心驱动的子会话；
  编辑器 UI 依赖不进入运行时。
- **headless**：核心测试不启动 UI；UI 后端只做冒烟测试。
- **ADR-0009**：M12 spike 后决定默认后端，评估 GPUI 的事件循环/纹理共享/许可证与稳定性；不可行则用 egui
  或自研最简宿主。切换后端不得改动核心逻辑。
- **不做**：Sequencer/Niagara、蓝图编辑器、内置 C# 代码编辑器（C# 用外部 IDE，见 §9.13）。
- 验收：核心 headless 测试全绿；选定 UI 后端可编辑/保存/PIE，视口由 vulkanalia 渲染。

### 9.16 `miriya-cli` / `mir-build` / `mir-pak`：工具链

- 命令：`miriya new`（模板项目）、`run`、`build`、`cook`、`pack`、`migrate`、`doctor`。
- Cook：只保留被引用资产，按平台配置裁剪；依赖图决定顺序；失败列出资产 ID 与引用链。
- `.mipak`：magic/version/索引/压缩块/完整性校验，mmap 读取；C# 程序集与 `.deps.json` 随包分发。
- 构建：Cargo profile + 资产管线 + 着色器预编译缓存 + `dotnet build`（Windows）；示例包体 ≤ 200 MB。
- 验收：干净机器双击可运行；增量打包 ≤ 10 s（小改动）；Linux 接口编译检查通过。

### 9.17 平台与网络

- 平台：**Windows 首发且当前唯一发布平台（Vulkan 1.3）**；Linux 接口保留并做编译检查，发布留待 N6；
  macOS/移动端/主机不做。
- 平台服务：文件（大小写/长路径/沙箱）、计时、DPI、电源模式、崩溃 minidump（后续）。
- 网络：N1 用固定步长 + 服务器权威快照；不承诺 UE 复制语义 1:1；先做专用服务器 headless。

---

## 10. 资产与数据格式

### 10.1 设计目标

- **稳定引用**：资产用 UUID 标识，移动/重命名不破坏引用。
- **可版本化**：每个容器带 schema 版本与构建哈希，支持迁移与增量 cook。
- **可校验**：magic + 长度 + 校验和；解析器可 fuzz，坏资产不崩溃。
- **零拷贝友好**：二进制负载按 GPU 对齐，mmap 直接读；元数据用 serde。

### 10.2 原生格式

| 文件/扩展 | 用途 | 编码 | 备注 |
|---|---|---|---|
| `Miriya.toml` | 项目清单 | TOML | 启动关卡、插件、平台配置、`[scripting]` C# 程序集 |
| `config/*.toml` | 引擎/游戏配置 | TOML | 替代 UE `.ini` |
| `.miasset` | 通用资产容器 | 头 + zstd 负载 | 网格/材质/纹理元数据共用容器，type tag 区分 |
| `.mimap` | 关卡 | 头 + 反射序列化 | Actor/组件/层级/流式单元 |
| `.mipak` | 烘焙包 | 索引 + 压缩块 | 运行时挂载，mmap |
| `.slang` / `.spv` | 着色器源/编译产物 | Slang / SPIR-V | `slangc` 直接编译；缓存按管线哈希 |
| `managed/*.csproj` | C# 脚本项目 | C# 源 + .NET 11 工程 | `dotnet build`；生成绑定来自 `mir-bindgen` |
| `.dll` / `.deps.json` | C# 运行时程序集 | .NET 程序集 | 随包分发；开发期可卸载 ALC 热重载 |
| 导入源 | glTF/GLB、PNG/KTX2、WAV/OGG/FLAC | 标准格式 | 构建期转换，不进入运行时 |

**容器头（草案）**：`MIRI` magic、`u16 version`、`u16 type`、`u32 flags`、`uuid`、`u64 payload_len`、
`u32 schema_hash`、压缩算法/块表、校验和。所有多字节字段小端；字符串表去重。

### 10.3 引用与依赖

- 资产间引用一律存 UUID（+ 可选子对象路径），运行时不存文件路径。
- 注册表维护反向依赖，cook 从启动关卡出发求闭包；循环引用在导入期报错并给出链路。
- 缺失引用是**构建错误**，不是运行时 panic；编辑器用占位资产提示。

### 10.4 迁移与 UE 资产

- 版本迁移工具 `miriya migrate`：按 schema 版本逐级升级，迁移前后跑 golden 校验。
- `mir-uasset`（feature-gated，实验）：只读解析 `.uasset/.umap/.pak` 供内容盘点/迁移辅助；
  不复制 UE 序列化实现，不承诺完整覆盖；是否随发行版提供需法律评审（§11.6）。
- 不实现 UE 资产运行时加载；不实现 `.uasset` 写入。

### 10.5 坐标与单位

- 资产内部与运行时统一 UE 约定（cm、Z-up、左手）；glTF 导入时做 Y-up→Z-up 与手性转换并记录在资产元数据。
- 导入报告列出被转换的节点、缩放与轴向，便于排查「模型歪了」。

### 10.6 C# 托管代码

- `Miriya.toml` 的 `[scripting]` 指定程序集、入口类型与热重载开关。
- `miriya build` 调用 `dotnet publish`；`miriya run` 由 `mir-script` 通过 hostfxr 加载程序集。
- 发布包包含自包含 .NET 11 运行时、用户程序集与 `.deps.json`；引擎与程序集通过版本化 C ABI 匹配，
  版本不匹配在启动时明确报错。
- 编辑器保存时触发增量构建；失败时保留上一份可运行程序集并提示错误。
- 不存在蓝图/可视化脚本资产格式；C# 程序集是唯一的脚本载体。

---

## 11. 工程规范与质量保障

### 11.1 工具链与语言

- 只使用**稳定版 Rust（stable）**；Edition 2024；`rust-toolchain.toml` 固定 stable 通道（本机解析为 1.100.0）；
  MSRV 以 CI 为准，且不低于 edition 2024 的最低要求（1.85）。nightly 仅限 fuzz/bench 等非发布工具。
- C# 侧固定 **.NET 版本**（当前基线 .NET 11，2026-11-23 M0 复核确认：.NET 11 与 .NET 10 终止支持日期相同，
  均为 2028-11-14，在支持周期等价时选用更新版本），`global.json` 锁定 SDK 版本；
  每个 M 里程碑复核一次支持周期，出现更早退役或支持终止的候选时升级。
  `dotnet format` 与 `dotnet test` 纳入 CI。
- `rustfmt` 统一格式；Clippy 启用 `correctness/suspicious/complexity` 并 `-D warnings`，按需 allow。
- 工作区统一 `[workspace.dependencies]` 与 `[workspace.lints]`；新增依赖需说明理由与许可证。

### 11.2 代码规范

- 命名：crate `mir-*`（引擎内部）/ `miriya-*`（门面、应用、工具）；类型 `UpperCamel`；函数/变量 `snake_case`。
- **Rust 惯用法**：参数用 `&T`/`&mut T`/`impl AsRef`/`Into`，返回 `Result`/`Option`；避免 bool 参数与 out 指针；
  用 newtype/enum 表达领域语义，禁止裸 `u32`/`bool` 跨模块传业务概念。
- 类型擦除（`dyn Any`/`Box<dyn Error>` 的滥用）只允许在反射、脚本与插件边界；业务路径优先具体类型/泛型。
- 除 FFI/Vulkan/音频/平台 crate 外，crate 级 `#![forbid(unsafe_code)]`；`unsafe` 集中在边界。
- 错误：库用 `thiserror` 定义领域错误，CLI/应用用 `anyhow`；运行时与资产解析禁止 `unwrap/expect/panic!`。
- C# 代码：遵循 .NET 约定（公共成员 PascalCase）、启用 `nullable`、禁止 `unsafe` 跨 ABI；生成的绑定不得手改。
- `unsafe` 必须有 `// SAFETY:` 注释与最小化作用域；新 unsafe 需评审。
- 模块按职责而非文件大小拆分；公共 API 必须有 rustdoc 与至少一个示例。
- 禁止提交 `dbg!`、调试打印、注释掉的死代码；临时实验走 feature flag。

### 11.3 测试与基准

| 层级 | 内容 | 工具 |
|---|---|---|
| 单元 | 数学、序列化、反射、容器、解析 | `cargo test` |
| 集成 | 场景存取、资产管线、输入映射、物理接口契约（mock 后端） | `cargo test --workspace` |
| 物理契约 | `PhysicsBackend` 全接口、生命周期、查询/事件、坐标转换 | mock 后端 + 外部后端可复用套件 |
| 编辑器核心 | 文档/命令/撤销/Inspector/PIE（无 UI 依赖） | `cargo test`（headless） |
| UI 后端 | GPUI/egui 冒烟、视口合成、事件映射 | 编辑器集成测试（可选） |
| 无头 GPU | 离屏 Vulkan 渲染 + 像素比对（容差） | 离屏 Vulkan；CI 无 GPU 时用 lavapipe 测试适配器（非发布平台） |
| Golden | 关卡/资产/渲染输出基准 | 自建 + `insta` 可选 |
| Fuzz | `.miasset/.mimap/.mipak`、导入器 | `cargo-fuzz` |
| C# 互操作 | 绑定、句柄生命周期、异常传播 | `dotnet test` + 边界用例 |
| 脚本 | C# 示例玩法、热重载、版本化 ABI | `dotnet build` + 引擎集成测试 |
| 基准 | ECS、物理接口、资产加载、帧时间、脚本开销 | `criterion` |
| 压力 | 10k 实体、1 GB 资产、长时间运行 | 夜间 CI |

CI 在 Windows 上跑 fmt/clippy/test 与 `dotnet build/test`；Linux 侧跑
`cargo check --target x86_64-unknown-linux-gnu`（只验证接口，不跑运行时）。
额外执行依赖方向检查：运行时 crate 不得依赖编辑器/UI 框架/具体物理后端。
Vulkan GPU 测试允许在无硬件时跳过并显式标注（不得静默通过）。

### 11.4 性能预算（持续跟踪）

| 指标 | 目标（1080p，中端独显） |
|---|---|
| 示例场景 GPU 帧时间 | ≤ 16.6 ms（60fps） |
| 主线程 CPU 帧时间（逻辑+提取） | ≤ 4 ms |
| 10k 实体变换更新 | ≤ 1 ms |
| 物理接口调度/查询 | ≤ 0.2 ms（不含外部后端求解时间） |
| 资产冷加载 | 主线程单帧阻塞 ≤ 16 ms |
| C# 脚本边界 | 每帧回调开销 ≤ 0.5 ms（示例场景） |
| UI 开销 | ≤ 1 ms |
| 示例包体 | ≤ 200 MB |

预算超标需在 PR 中给出回归报告与豁免理由，并用基准结果更新本文档。

### 11.5 诊断与可观测性

- `mir-log` + `tracing` 桥接：结构化日志、span 覆盖帧/资产/渲染 pass。
- 帧统计：CPU/GPU 时间、draw call、实例数、内存、资产驻留；编辑器面板与 CLI `--stats` 共用。
- Vulkan：validation layer、timestamp queries、debug marker；`tracy`/`puffin` 作为可选 feature。
- 物理：后端上报步进/查询耗时与调试形状；引擎只统计接口开销。
- 编辑器：`mir-editor-core` 的帧耗时与命令耗时计入统计；UI 后端各自上报。
- C# 诊断：托管异常栈映射到 C# 源码；脚本耗时与调用次数计入帧统计。
- 崩溃：发布版捕获 panic hook + minidump（后续）与 C# 未处理异常，日志落盘；不允许静默退出。

### 11.6 许可与合规（重要）

- 代码拟采用 **MIT OR Apache-2.0** 双许可；依赖通过 `cargo-deny` 审核（禁 GPL/未知许可）。
- .NET 运行时与 NuGet 依赖单独审核（与 MIT/Apache 兼容、版本锁定、生成 SBOM）；C# 用户程序集归用户所有。
- 编辑器 UI 框架（GPUI/egui）的许可证、传递依赖与 API 稳定性在 ADR-0009 中评审；任何 UI 框架都不得进入运行时依赖。
- **clean-room 声明**：Miriya 不复制、不翻译 UE 源代码；不将 UE 头文件/实现细节作为设计输入；
  仅依据公开文档、API 参考与可观察行为。任何从 UE 源码「移植」的 PR 一律拒绝。
- UE EULA 对源码使用、衍生作品与资产有严格限制；`mir-uasset` 等接触 UE 专有格式的功能
  发布前必须由法律顾问确认，必要时仅保留内部工具、不随发行版分发。
- 不使用 Unreal/UE 商标、Logo、模板内容作为 Miriya 的命名或宣传；示例内容自制或使用许可兼容资产。
- 本文不构成法律意见；正式发布前需完成合规评审。

### 11.7 文档与决策

- `docs/book/` 用 mdbook：快速开始、概念（Actor/资产/渲染/C# 脚本）、物理后端接入、编辑器 UI 后端、迁移指南、贡献指南。
- `docs/adr/NNNN-*.md` 记录架构决策（模板：背景/决策/替代方案/后果）。
- `docs/rfcs/` 承载大特性提案；跨 crate 或影响公共 API 的改动必须先有 ADR/RFC。
- C# API 文档由 XML 注释生成并与 rustdoc 一并发布；C ABI 表有版本与兼容性说明。

### 11.8 版本与发布

- 0.x 阶段按语义化版本预发布（`v0.4.0` 对应垂直切片）；破坏性变更写 CHANGELOG。
- 发布流程：CI 全绿 → 基准无回归 → Windows/Vulkan 冒烟 → 打 tag → 发布说明。
- 发布产物为 Windows x64 自包含包（含 .NET 11 运行时）；Linux 只发布接口编译检查结果，不发布运行时。
- 每个里程碑结束更新本 README 的进度表与「当前进度」。

---

## 12. 风险与应对

| 风险 | 概率 | 影响 | 应对 |
|---|---|---|---|
| 范围失控（想对齐 UE 全部功能） | 高 | 高 | 同心环 + 里程碑 Gate；非目标写死；每个版本只做垂直切片 |
| 渲染现代化差距（Lumen/Nanite） | 高 | 中 | 明确不做；先稳定 Forward+；研究项独立分支，不影响主线 |
| Vulkan 驱动/硬件覆盖（无 DX12 兜底） | 中 | 高 | 明确 Vulkan 1.3 系统要求；NVIDIA/AMD/Intel 矩阵测试；validation layer + `spirv-val`；设备丢失恢复路径；自研分配器做泄漏/碎片测试 |
| 着色器/驱动差异导致画面不一致 | 中 | 中 | SPIR-V 校验 + 多 GPU golden 测试；管线缓存按驱动指纹；可诊断的降级开关 |
| C#/.NET 互操作、GC 与热重载 | 中 | 高 | 版本化 C ABI、生成绑定、句柄世代校验、边界测试；热重载失败回退 PIE；脚本性能基准 |
| 资产格式与迁移成本 | 中 | 中 | 自有格式早期定型；导入器优先 glTF；fuzz + 版本迁移工具 |
| ECS/反射性能不达预期 | 中 | 中 | hecs + 基准门禁；反射宏按需生成；保留替换路径 |
| 外部物理后端的契约/集成/确定性 | 中 | 高 | 明确 trait 契约与可复用契约测试；`NullPhysicsBackend` 兜底 CI；坐标/生命周期/序列化文档化；物理性能由后端负责 |
| 编辑器 UI 框架选型（GPUI/egui）与事件循环/纹理共享 | 中 | 高 | `mir-editor-core` 零 UI 依赖；M12 先 spike；离屏纹理/子表面适配，必要时回退 egui；ADR-0009 记录许可证与 API 稳定性 |
| 编辑器工作量被低估 | 高 | 高 | 核心/UI 解耦、功能裁剪；先做搭关卡必需工具；其余后置 |
| 长期维护与人员流失 | 高 | 高 | 文档/ADR/测试齐全；小依赖面；每子系统有 owner 与入门任务 |
| 法律与 EULA 风险 | 中 | 高 | clean-room 流程；UE 资产功能法律评审；不使用其商标与内容 |
| Linux 接口长期不运行而锈化 | 中 | 中 | CI 交叉 `cargo check` + 平台 trait 契约测试；N6 前定期冒烟 |
| 性能回归悄无声息 | 中 | 中 | criterion 基准 + 夜间压测 + 预算表；PR 必须附基准对比 |
| 单人维护精力不足 | 高 | 中 | 估算按 3–4 人；单人模式缩小环 1 范围（先 v0.4）再推进 |

---

## 13. 验收标准

### 13.1 v1.0 发布门槛（全部满足）

**功能**

- [ ] Windows x64（Vulkan 1.3）上，用 Miriya 从零搭建并运行一个第一人称/第三人称示例
      （移动、跳跃、射击、HUD、音效；物理行为由外部后端提供），玩法逻辑由 C# 编写；
      Linux 仅要求平台接口通过编译检查。
- [ ] C# 脚本：`miriya build` 可编译、宿主可加载、PIE 中热重载 ≤ 1 s、发布包无需安装 .NET SDK 即可运行；
      项目中不存在蓝图/可视化脚本。
- [ ] 编辑器可创建/保存/加载关卡，Inspector 可编辑反射属性，PIE 一键运行。
- [ ] 资产导入（glTF/PNG/KTX2/WAV/OGG）与热重载可用；cook + `.mipak` + C# 程序集 + 独立可执行文件可用。
- [ ] 物理：`PhysicsBackend` 接口、固定步调度、mock 后端契约测试通过；外部自研后端可接入；
      引擎与仓库内**不含**物理求解器/角色控制器实现。
- [ ] 编辑器：`mir-editor-core` 的 headless 测试通过；选定 UI 后端（GPUI 或 egui）可编辑/保存/PIE；
      切换 UI 后端不需要修改核心逻辑。
- [ ] 文档：快速开始 30 分钟内跑通示例；Rust 与 C# API 文档无缺失的公共项。

**质量**

- [ ] CI 全绿（fmt/clippy/test/`dotnet build/test`/基准冒烟/资产 fuzz 抽样）；Vulkan 校验层无错误；
      无 P0/P1 已知缺陷。
- [ ] 公共 API/模块边界通过 Rust 惯用法评审（§4.2）：不存在 1:1 复刻的 C++ 继承、裸指针所有权或
      `UObject` 布局模拟；业务路径无 `dyn Any` 滥用。
- [ ] 连续运行 8 小时无崩溃、无持续内存增长（示例场景）。
- [ ] 损坏资产、缺失依赖、Vulkan 设备丢失、C# 未处理异常均有可读错误与恢复路径。

**性能**

- [ ] §11.4 预算全部达标并记录基准机、GPU、Vulkan 驱动版本。
- [ ] 示例场景 1080p 60fps；编辑器视口同场景 ≥ 30fps；C# 脚本开销在预算内。

### 13.2 早期切片的验收

- **v0.2**：窗口可关闭、相机可自由飞行、立方体带正确深度与光照；Vulkan 校验层无错误。
- **v0.4**：一条完整玩法链路可玩 5 分钟不崩溃；C# 驱动移动/射击，输入/音频/UI 协同正常；
  物理行为需先接入外部物理后端（未就绪时仅验证接口契约）。
- **v0.5**：编辑器中搭一个房间并 PIE，保存后重新打开内容一致；C# 热重载可用。
- **v0.6**：把示例复制到干净机器（无 Rust/C# 工具链）可直接运行。

---

## 14. 当前进度与快速开始

### 14.1 已完成

- [x] workspace 已按目标结构拍平并收敛：**41 个成员**（37 个 crates + 4 个 tools，含 `mir-vulkan-probe` PoC）；
  11 个引擎子 crate 已有实现，27 个空壳 crate 仅含模块文档。
- [x] `rust-toolchain.toml`、`.editorconfig`、`managed/`、`docs/`、`assets/`、`config/`、`shaders/`、`tests/` 骨架。
- [x] 基础主循环（`AppState` 回调、帧率限制、时间推进）。
- [x] 数学 newtype 封装（Vec/Mat/Quat）与单元测试。
- [x] 纯 std 日志系统（五级宏、全局 logger、控制台彩色输出、测试）。
- [x] ECS/资产/场景/输入/窗口/渲染的最小原型与占位实现。
- [x] 规划文档（本 README）已按 **Windows-only + Vulkan/vulkanalia + C# 脚本 + 物理仅接口 + 编辑器 UI 解耦 +
  不做 1:1 迁移/Rust 优先** 的新约束更新。
- [x] 门面 `miriya-engine` 已 `pub use mir_log as log`，应用层可访问日志 API。
- [x] 补齐工程基础设施：`LICENSE-MIT`、`LICENSE-APACHE`、`global.json` 与基础 GitHub Actions CI
  （Windows: fmt/clippy/test + .NET 11 构建；Linux: `cargo check`）。
- [x] 主循环支持 `max_frames`，示例 `miriya-app` 可通过 `--frames N` 自动退出，不再只能 Ctrl+C 强杀。
- [x] 渲染/窗口子系统文档注释已对齐唯一后端 `vulkanalia` / `winit` 规划。
- [x] **M1 平台与循环**：`mir-window` 接入 winit 0.30 真实窗口与事件循环；键位映射覆盖字母/数字/F1–F12/
  方向键/修饰键；`mir-input` 补全鼠标按键沿与失焦清理；窗口关闭、Esc、`max_frames` 均可退出主循环。
- [x] 帧统计 `FrameStats`（帧数、平滑 FPS、帧耗时）接入 `EngineContext`；`Engine::run_headless` 支持
  无窗口跑帧，新增 `headless_1000_frames` 测试。
- [x] M0 遗留：`vulkanalia` PoC 全链路实测通过（RFC 0001），ADR 0001–0012 已落盘 `docs/adr/`。
- [x] **M0 契约起草**：`mir-physics` 落地 `PhysicsBackend` 契约、`BodyId`/`ColliderId` 世代句柄、
      `FixedStep`、射线/扫掠/重叠/接触事件/调试绘制数据、`NullPhysicsBackend` 与可复用的
      `assert_backend_contract`；`mir-editor-core` 落地 UI 无关的关卡文档/选择/类型化命令与撤销栈/
      Inspector 字段描述/视口与 Gizmo 数学/PIE 状态机，并附 headless 测试。两 crate 均 `forbid(unsafe_code)`，
      不依赖 winit/Vulkan/UI 框架。
- [x] **M2 第一阶段**：`vulkanalia` 已进入 `mir-render`——Vulkan 1.3 实例、物理设备筛选、逻辑设备与
      图形/呈现队列、可选 surface 与交换链前置查询（格式/呈现模式）；公共 API 不含 Vulkan 类型，
      headless 初始化在 CI/无 GPU 环境下降级而非 panic。`RenderContext` 既有签名保持不变。
- [x] **M2 第二阶段**：`mir-window::Window` 实现 `HasWindowHandle`/`HasDisplayHandle`，新增
      `WindowApp::on_window_created` 回调；`mir-core` 在窗口创建后用真实句柄初始化渲染；
      `mir-render` 创建交换链与图像视图（优先 IMMEDIATE，回退 FIFO；OPAQUE 合成），
      并按 image views → swapchain → surface → device → instance 逆序释放。窗口冒烟 `--frames 60` 通过。
- [x] **M2 第三阶段**：`mir-render` 用 Vulkan 1.3 动态渲染完成每帧清屏，并经命令缓冲提交、`acquire →
      present` 真正呈现到窗口；单帧在飞行（fence + 双 semaphore）同步，窗口缩放时重建交换链。
      `frame_error()` 暴露帧级错误，示例应用退出时输出；`cargo run -- --frames 60` 无渲染错误。
- [x] **M2 第四阶段（v0.1 Hello GPU）**：`shaders/hello/triangle.slang` 单文件包含顶点/片元入口，
      经 Slang 编译与 `spirv-val` 校验的 SPIR-V 落盘；`mir-render` 创建着色器模块与动态渲染图形管线，
      用 `SV_VertexID` 绘制彩色三角形（无顶点缓冲）。交换链格式变化时重建管线；`cargo run -- --frames 60` 无渲染错误。
- [x] **M2 第五阶段（v0.2 Cube 达成）**：深度缓冲、交换链 layout/失效恢复、staging 网格上传与世代句柄、
  `shaders/cube/cube.slang`（UBO + push constants）、逐实例绘制 API 已落地；演示场景为 3×3 阵列 + 中心自转，
  共 10 个实例。
- [x] **自由相机与输入**：`mir-scene::{Camera, CameraController}` + WASD/QE/Shift + 右键鼠标视角；
  `--width` / `--height` / `--validation` / `--no-validation` 可用，失焦自动释放光标。
- [x] **校验层代码内接入**：`EngineConfig::validation` 控制 layer 探测、`VK_EXT_debug_utils` messenger 与 `mir-log`
  桥接；layer 缺失时警告降级。release 构建曾暴露 `WriteDescriptorSet` 临时数组悬垂指针，已修复并纳入
  `docs/plans/v0.2-cube.md` §10。
- [x] **ADR-0012 与 CI**：记录 extract 边界、UBO/push constants 划分、单帧在飞行取舍与演示单位偏差；
  CI 两个 job 均重新生成并校验 `shaders/hello/*.spv`、`shaders/cube/*.spv`；Clippy 收紧为 `-D warnings`。

### 14.2 已知问题（按里程碑优先修复）

1. 27 个空壳 crate 尚未实现；`miriya-engine` 目前只 re-export 既有原型子系统，新模块尚未接入 facade。
2. ECS 为 `HashMap<TypeId, HashMap<Entity, Box<dyn Any>>>` 原型：无查询/调度/变更检测；`Entity` 是裸 `u64`，
   无世代标记，后续若回收 ID 会产生悬垂句柄。
3. `mir-render` 已具备深度、网格、逐实例绘制、校验层、多帧在飞行、RenderGraph v0.5、单 `.spv` 多入口与
   持久化管线缓存，但尚无材质/PBR、自研显存子分配器与视锥剔除（M2 批次 E/F）。
4. `mir-asset` 加载失败直接 `panic`；异步加载与错误化在 M4 处理。
5. Git 仓库已初始化且已推送到 `origin/main`；本地工作区需随每次里程碑同步提交。
6. DPI 缩放尚未处理；窗口尺寸仅以物理像素记录，未适配高 DPI 显示器。
7. `vulkanalia` 已接入 `mir-render` 且校验层可用；.NET 11 宿主仍未接入；
   `mir-script`、`mir-bindgen`、`managed/` 仍为空壳，需 M8 落地。
8. 当前没有物理求解器（这是新约束）：`mir-physics` 已提供可编译的契约骨架与 `NullPhysicsBackend`，
   但求解器仍须由外部自研后端提供；引擎侧固定步/句柄表/坐标转换胶水留待 M6，v0.4 起需接入外部后端。
9. 编辑器 UI 后端尚未选定：`mir-editor-core` 已有 UI 无关契约骨架（headless 测试通过），
   `mir-editor-ui`/`mir-editor-gpui`/`mir-editor-egui` 仍为空壳；GPUI/egui 需在 M12 前完成 spike 与 ADR-0009。
10. Vulkan builder 只保存切片指针，临时数组在 release 下可能失效；本次已修复一处，后续 unsafe 审查需持续检查
    同类模式，并把 release + 校验层运行纳入重要渲染改动的验收。

### 14.3 快速开始

```bash
# 需要 Rust stable（rust-toolchain.toml 固定为 stable，本机解析为 1.100.0）
cargo check --workspace
cargo test --workspace

# 运行示例（3×3 立方体 + 中心自转，10 实例；WASD/QE 移动、右键拖动视角、Esc 退出）
# workspace 已设置 default-members，根目录直接 cargo run 即可
cargo run
cargo run -- --frames 600 --width 1920 --height 1080
cargo run --release -- --frames 600 --width 1920 --height 1080

# 无窗口跑帧（CI/headless 验证）；校验层 debug 默认开启，可显式覆盖
cargo run -- --headless --frames 1000 --no-validation
cargo run -- --validation --frames 120

# Vulkan 链路探测（M0 PoC，需 Vulkan 1.3 驱动）
cargo run -p mir-vulkan-probe

# 修改 *.slang 后，重新编译 SPIR-V（需 Slang 编译器，可由 Vulkan SDK 或 Slang release 提供）
# 单 .spv 多入口：-fvk-use-entrypoint-name 保留 vertexMain/fragmentMain 入口名
slangc shaders/hello/triangle.slang -profile glsl_450 -target spirv -fvk-use-entrypoint-name -o shaders/hello/triangle.spv
slangc shaders/cube/cube.slang -profile glsl_450 -target spirv -fvk-use-entrypoint-name -o shaders/cube/cube.spv
spirv-val shaders/hello/triangle.spv
spirv-val shaders/cube/cube.spv
```

`cargo check --workspace`、`cargo test --workspace`、`cargo fmt --all -- --check` 与
`cargo clippy --workspace --all-targets -- -D warnings` 当前通过（仅 Cargo 清单有既有 unused dependency 警告）；
窗口由 winit 创建，关闭窗口或按 Esc 均通过 `request_exit` 退出；`Engine::run_headless` 不创建事件循环。
v0.2 Gate 结果与 release 构建中发现并修复的悬垂指针问题见 `docs/plans/v0.2-cube.md` §9/§10。
M8 起还需要 .NET 11 SDK 编译 C# 脚本。

### 14.4 下一步（M2 剩余与后续里程碑）

- [x] M0：workspace 拍平（41 个成员）、工具链/许可/`global.json`/CI、ADR 0001–0012、
      `vulkanalia` PoC、物理与编辑器核心契约、`max_frames` + headless 1000 帧。
- [x] M1：winit 窗口与事件循环、键鼠映射、输入快照、鼠标增量/光标捕获、帧统计、退出链路。
- [x] M2 / v0.2 Cube：深度、交换链健壮性、网格上传、cube 着色器与管线、10 实例提交、自由相机、
      代码内校验层 + `mir-log` 桥接、release 悬垂指针修复、CI 双 shader 校验；详见
      `docs/plans/v0.2-cube.md` §9/§10。
- [ ] **M2 剩余**：RenderGraph v0.5、2–3 帧在飞行与环形 UBO、单 `.spv` 多入口、持久化管线缓存
      ✅ 已完成（批次 A–D，2026-11-24）；尚余自研显存子分配器（批次 E）、网格/材质系统与基础 PBR
      （批次 F，v0.3 前置）；任务清单与验收 Gate 见 `docs/plans/m2-remaining.md`。
- [ ] **M3**：hecs 封装、Actor/Component、层级变换、序列化往返；`Transform` 从 `mir-scene` 收敛到 `mir-math`
      并按 ADR-0012 校准演示单位（米 → 厘米）。
- [ ] **M4–M13**：按 §8.2 依次推进资产管线、光照、物理接口胶水、游戏框架、C# 脚本、音频、动画、UI、编辑器与打包。
- [ ] 本 README 的进度表接入里程碑看板（issue/项目面板）；每次里程碑结束同步 §14 与 `docs/plans/`。

---

## 15. 参考与术语

### 15.1 参考

- Unreal Engine 源码（本机）`../UnrealEngine`，版本 5.8.4（仅行为参考；许可见 §11.6）。
- UE 官方文档：<https://dev.epicgames.com/documentation/unreal-engine>
- Rust API Guidelines：<https://rust-lang.github.io/api-guidelines/>
- Vulkan 规范：<https://docs.vulkan.org/> · vulkanalia：<https://github.com/KyleMayes/vulkanalia>
- Slang：<https://github.com/shader-slang/slang> · SPIR-V：<https://www.khronos.org/spir/>
- .NET 11：<https://dotnet.microsoft.com/> · hostfxr 托管：<https://learn.microsoft.com/dotnet/core/tutorials/netcore-hosting>
- winit：<https://github.com/rust-windowing/winit> · glam：<https://github.com/bitshifter/glam-rs>
- hecs：<https://github.com/Ralith/hecs> · glTF 2.0：<https://www.khronos.org/gltf/>
- GPUI（编辑器 UI 候选）：<https://github.com/zed-industries/zed/tree/main/crates/gpui>
- egui（编辑器 UI 备选）：<https://github.com/emilk/egui>
- KTX2：<https://www.khronos.org/ktx/> · cpal：<https://github.com/RustAudio/cpal>
- symphonia：<https://github.com/pdeljanov/Symphonia> · tracing：<https://tokio.rs/tokio/topics/tracing>
- 物理后端：Miriya 只提供 `PhysicsBackend` 契约（见 §9.9）；具体求解器由用户自研，不在本仓库。

### 15.2 术语

| 术语 | 含义 |
|---|---|
| 1:1 迁移 | 逐文件/逐类复刻 UE 结构；**本项目明确不做**，只做语义与工作流对齐 |
| Rust 优先 | 在语义等价时优先所有权/newtype/enum/trait/Result 等 Rust 惯用法（§1.3/§4.2） |
| Actor / Component | 关卡中的对象及其可组合能力；Miriya 用「实体 + 组件 + 元数据」实现 |
| PIE | Play In Editor，编辑器内运行游戏 |
| Vulkan / vulkanalia | Miriya 唯一的图形后端；vulkanalia 是 Rust 的 Vulkan 绑定与辅助层 |
| Slang | Miriya 的唯一着色器源语言；支持 module/import，经 `slangc` 直接生成 SPIR-V |
| SPIR-V | Vulkan 使用的着色器中间格式；Miriya 由 Slang 经 `slangc` 直接编译得到 |
| C# / .NET 11 | 唯一的玩法脚本语言与运行时；通过 hostfxr 在引擎进程内托管 |
| PhysicsBackend | 外部物理后端契约；Miriya 只提供接口、调度与契约测试，不含求解器 |
| mir-editor-core / UI 后端 | 编辑器核心（UI 无关）与可插拔 UI 适配层（GPUI/egui），后者不进入运行时 |
| GPUI / egui | 编辑器 UI 的候选/备选框架；由 ADR-0009 定选，核心逻辑不依赖它们 |
| hostfxr / ALC | .NET 宿主入口与可卸载程序集加载上下文（AssemblyLoadContext） |
| 蓝图 / Blueprint | UE 的可视化脚本系统；Miriya 明确不实现，脚本统一用 C# |
| UHT | Unreal Header Tool，UE 的反射代码生成器（Miriya 用 derive 宏） |
| Cook | 把源资产转换为平台运行时格式的烘焙过程 |
| Pak | 打包后的资产归档（Miriya 为 `.mipak`） |
| Gizmo | 编辑器中的移动/旋转/缩放操纵器 |
| CSM / IBL / SSAO / DDGI | 级联阴影 / 基于图像光照 / 屏幕空间环境光遮蔽 / 动态漫反射全局光照 |
| 垂直切片 | 从输入到呈现完整打通的一小条功能链路，可独立演示与验收 |

---

## 16. 如何参与

1. **先读**：本 README → `docs/book/` 概念篇 → 目标 crate 的 rustdoc、C# API 文档与 ADR。
2. **选题**：优先看 §14.4 的 M0 清单与里程碑 Gate；新手任务标注 `good first issue`。
3. **架构变更**：涉及公共 API、crate 边界、资产格式、坐标约定者，先提 RFC/ADR，再写代码。
   **不做 1:1 C++ 移植；API 与实现优先 Rust 惯用法（§4.2）。平台/图形/脚本/物理/编辑器约束已定：
   Windows-only（Linux 接口保留）、唯一 Vulkan 后端（vulkanalia）、C# 脚本（无蓝图）、物理只留接口
   （外部后端）、编辑器核心与 UI 框架解耦；不接受引入 wgpu/ash/DX12、第二脚本系统或内置物理求解器。**
4. **提交**：Conventional Commits；PR 必须含测试、文档与基准（如涉及性能）；C# 变更同样需要测试与评审。
5. **评审**：每个 PR 至少一名非作者评审；渲染/物理接口/资产解析/C# 互操作/编辑器核心变更需对应 owner 复核；
   公共 API 需通过 Rust 惯用法检查（无 C++ 式样板、无多余 `dyn`/`unsafe`）。
6. **同步**：每个里程碑结束更新本文件（进度表、已知问题、下一步），保持计划与代码一致。

---

> **文档维护**：Miriya 团队 · 最后更新 2026-11-23
> 本文件是**规划**，不代表上述能力已经完成；进度以 §14 为准，变更以 ADR 为准。
