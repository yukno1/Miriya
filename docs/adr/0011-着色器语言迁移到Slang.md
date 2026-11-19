# ADR 0011：着色器语言迁移到 Slang

- 日期：2026-11-19
- 状态：Accepted
- 关联：README §6.2、§8.2（M2/M5/M13）、§9.6、§10.2、§14；ADR 0004

## 背景

ADR 0004 原定使用 GLSL 4.60 配合 `shaderc` 生成 SPIR-V。v0.1 Hello GPU 落地后，着色器源码仅有
`shaders/hello/triangle.{vert,frag}`，运行时不解析源码，只通过 `vulkanalia::include_shader_code!`
嵌入已编译的 SPIR-V。随着 M5 材质/光照引入公共库和多 pass，GLSL 的自研 include 与跨目标复用成本偏高。
Slang 提供 module/import、泛型与抽象，并可直接生成 SPIR-V，适合在本项目单 Vulkan 后端下统一着色器工程。

## 决策

1. **唯一着色器源语言**：所有新着色器使用 `.slang`，停止新增 GLSL 源码；仓库内示例 GLSL 源由 Slang 源替换。
2. **目标格式不变**：继续使用 Vulkan 1.3 + SPIR-V；`slangc -target spirv` 直接生成字节码，不经 DXC/glslang。
3. **入口约定**：单个 `.slang` module 可包含 `[shader("vertex")]` / `[shader("fragment")]` 两个入口函数
   （当前为 `vertexMain` / `fragmentMain`），分别编译到两个 `.spv`；Slang 默认把 SPIR-V 入口名写为 `main`，
   与当前 `mir-render` 的 `.name(b"main\0")` 保持一致。语义使用 `SV_VertexID`、`SV_Position`、
   `TEXCOORD0`、`SV_Target`，并校验 location 与管线输入一致。
4. **离线预编译**：`.spv` 继续提交进仓库并由 `include_shader_code!` 在编译期嵌入；运行时不引入 Slang 编译器。
5. **工具链入 CI**：Windows/Linux CI 安装固定版本 `slangc`，从 `.slang` 重新生成 `.spv`，并用
   `git diff --exit-code` 检查提交产物与源码一致。`slangc` 默认执行 SPIR-V 校验；本地可另跑 `spirv-val`。
6. **后续归属**：M2 剩余工作保持当前加载方式；M5 的公共代码用 Slang module/import 组织；M13 的 shader 预编译
   与缓存由 `mir-build`/`miriya-cli` 收编。
7. ADR 0004 的渲染管线、RenderGraph 与帧结构决策不变，仅第 5 条着色器语言决策由本 ADR 取代。
8. **单 `.spv` 多入口延后**：当前继续按阶段输出两个 `.spv`，保持 `mir-render` 的入口名约定不变；
   待 M2 RenderGraph 重构时一并改为单个 shader module 多 entry，届时配合 `-fvk-use-entrypoint-name`
   并调整 Rust 侧各阶段的 `.name(...)` 绑定。

## 替代方案

- **保留 GLSL + `shaderc`**：工具链成熟，但公共库仍需自研 include，难以复用抽象与跨 API 目标。
- **运行时 `shaderc`/`slangc` 编译**：开发期灵活，但增加运行时依赖、启动延迟与分发体积，违背当前离线模式。
- **手写 SPIR-V**：可完全控制产物，但可维护性差，不适合材质与多 pass 扩展。
- **HLSL + DXC**：DXC 生态成熟，但本项目只交付 Vulkan，且 Slang 的 module 与 SPIR-V 直出更贴合单后端策略。

## 后果

- 正面：着色器具备模块化表达能力；Vulkan 运行时与 SPIR-V 加载链路不变；CI 可校验源码与产物一致性。
- 负面：着色器作者需要 Slang 工具链（Vulkan SDK 自带或独立 release）；现有 GLSL 公共 chunk 需改写为 module。
- 准入：v0.1 的 `triangle.slang` 中两个入口须分别编译为入口 `main` 的顶点/片元 SPIR-V，
  `spirv-val` 通过，且 `cargo run -- --frames 60` 无渲染错误。
