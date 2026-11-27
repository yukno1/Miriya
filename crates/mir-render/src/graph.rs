//! RenderGraph v0.5（M2 剩余批次 B，R-03/R-04/R-05）。
//!
//! - **R-03 数据模型**：声明式 pass/资源 API，纯数据类型，不出现 `vulkanalia` 的
//!   `vk::*`（ADR-0012 公共边界；本模块为 crate 私有，`vulkan.rs` 每帧构建并消费）。
//! - **R-04 图编译器**：按资源读写声明生成数据流边（RAW/WAR/WAW），Kahn 拓扑排序
//!   验证无环，随后逐资源做布局追踪并推导 synchronization2 barrier/layout 转换列表。
//! - **R-05 增强**：临时（transient）资源的别名槽推导（生命周期不重叠即复用）与
//!   依赖图 DOT dump（`dump_dot`，由 CLI 开关控制每帧落盘）。
//!
//! v0.5 语义约定：
//! - pass 按**声明顺序**为期望执行序；编译器以数据流边验证一致性，
//!   同一资源上「先读后写」的跨 pass 对会被判定为数据流矛盾（环）。
//! - imported 资源的初始布局由导入方声明（如深度图像创建时已一次性转入
//!   `DEPTH_ATTACHMENT_OPTIMAL`），帧末按声明的 final 布局生成收尾 barrier
//!   （如交换链图像 → `PRESENT_SRC_KHR`）。
//! - buffer 不参与布局追踪（无图像布局），仅用于数据流边与 dump 展示；
//!   网格上传的 staging barrier 保留在独立传输路径（R-04 决策）。

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

use thiserror::Error;

use crate::texture::ImageFormat;

/// pass 在图中的句柄（下标即声明顺序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PassId(pub(crate) usize);

/// 资源在图中的句柄（下标即声明顺序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ResourceId(pub(crate) usize);

/// 图像布局（编译器追踪的最小集合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageLayout {
    Undefined,
    ColorAttachment,
    DepthAttachment,
    Present,
}

impl ImageLayout {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Undefined => "UNDEFINED",
            Self::ColorAttachment => "COLOR_ATTACHMENT",
            Self::DepthAttachment => "DEPTH_ATTACHMENT",
            Self::Present => "PRESENT",
        }
    }
}

/// 图像子资源 aspect（v0.5 只有颜色与深度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageAspect {
    Color,
    Depth,
}

/// 管线阶段位集（ synchronization2 语义的纯数据版本）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stages(u16);

impl Stages {
    #[allow(dead_code)] // 语义完整性；当前 barrier 推导以 TOP_OF_PIPE 作空源阶段。
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const TOP_OF_PIPE: Self = Self(1 << 0);
    pub(crate) const EARLY_FRAGMENT_TESTS: Self = Self(1 << 1);
    pub(crate) const LATE_FRAGMENT_TESTS: Self = Self(1 << 2);
    pub(crate) const COLOR_ATTACHMENT_OUTPUT: Self = Self(1 << 3);
    pub(crate) const BOTTOM_OF_PIPE: Self = Self(1 << 4);
    pub(crate) const TRANSFER: Self = Self(1 << 5);
    pub(crate) const VERTEX_INPUT: Self = Self(1 << 6);

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// dump 用的可读名称（多阶段以 `|` 连接）。
    pub(crate) fn label(&self) -> String {
        let mut names = Vec::new();
        if self.contains(Self::TOP_OF_PIPE) {
            names.push("TOP_OF_PIPE");
        }
        if self.contains(Self::EARLY_FRAGMENT_TESTS) {
            names.push("EARLY_FRAGMENT_TESTS");
        }
        if self.contains(Self::LATE_FRAGMENT_TESTS) {
            names.push("LATE_FRAGMENT_TESTS");
        }
        if self.contains(Self::COLOR_ATTACHMENT_OUTPUT) {
            names.push("COLOR_ATTACHMENT_OUTPUT");
        }
        if self.contains(Self::BOTTOM_OF_PIPE) {
            names.push("BOTTOM_OF_PIPE");
        }
        if self.contains(Self::TRANSFER) {
            names.push("TRANSFER");
        }
        if self.contains(Self::VERTEX_INPUT) {
            names.push("VERTEX_INPUT");
        }
        if names.is_empty() {
            "NONE".to_string()
        } else {
            names.join("|")
        }
    }
}

/// 访问掩码位集（ synchronization2 语义的纯数据版本）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Access(u16);

impl Access {
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const COLOR_ATTACHMENT_WRITE: Self = Self(1 << 0);
    pub(crate) const DEPTH_STENCIL_ATTACHMENT_WRITE: Self = Self(1 << 1);
    pub(crate) const SHADER_READ: Self = Self(1 << 2);
    pub(crate) const TRANSFER_WRITE: Self = Self(1 << 3);
    pub(crate) const VERTEX_ATTRIBUTE_READ: Self = Self(1 << 4);

    #[allow(dead_code)] // 语义完整性；v0.5 barrier 的访问掩码均单项使用。
    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// dump 用的可读名称（多项以 `|` 连接）。
    #[allow(dead_code)] // 仅 dump 场景使用；v0.5 dump 未标注 access。
    pub(crate) fn label(&self) -> String {
        let mut names = Vec::new();
        if self.contains(Self::COLOR_ATTACHMENT_WRITE) {
            names.push("COLOR_ATTACHMENT_WRITE");
        }
        if self.contains(Self::DEPTH_STENCIL_ATTACHMENT_WRITE) {
            names.push("DEPTH_STENCIL_ATTACHMENT_WRITE");
        }
        if self.contains(Self::SHADER_READ) {
            names.push("SHADER_READ");
        }
        if self.contains(Self::TRANSFER_WRITE) {
            names.push("TRANSFER_WRITE");
        }
        if self.contains(Self::VERTEX_ATTRIBUTE_READ) {
            names.push("VERTEX_ATTRIBUTE_READ");
        }
        if names.is_empty() {
            "NONE".to_string()
        } else {
            names.join("|")
        }
    }
}

/// pass 对资源的单次访问方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Usage {
    ColorWrite,
    DepthWrite,
    BufferRead,
    BufferWrite,
}

impl Usage {
    fn is_write(self) -> bool {
        matches!(
            self,
            Self::ColorWrite | Self::DepthWrite | Self::BufferWrite
        )
    }

    /// 该用法要求的图像布局；buffer 用法为 `None`。
    fn layout(self) -> Option<ImageLayout> {
        match self {
            Self::ColorWrite => Some(ImageLayout::ColorAttachment),
            Self::DepthWrite => Some(ImageLayout::DepthAttachment),
            Self::BufferRead | Self::BufferWrite => None,
        }
    }

    /// 该用法对应的（阶段，访问）组合。
    fn stage_access(self) -> (Stages, Access) {
        match self {
            Self::ColorWrite => (
                Stages::COLOR_ATTACHMENT_OUTPUT,
                Access::COLOR_ATTACHMENT_WRITE,
            ),
            Self::DepthWrite => (
                Stages::EARLY_FRAGMENT_TESTS.union(Stages::LATE_FRAGMENT_TESTS),
                Access::DEPTH_STENCIL_ATTACHMENT_WRITE,
            ),
            Self::BufferRead => (Stages::VERTEX_INPUT, Access::VERTEX_ATTRIBUTE_READ),
            Self::BufferWrite => (Stages::TRANSFER, Access::TRANSFER_WRITE),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::ColorWrite => "write(color)",
            Self::DepthWrite => "write(depth)",
            Self::BufferRead => "read",
            Self::BufferWrite => "write",
        }
    }
}

/// pass 声明。
#[derive(Debug, Clone)]
pub(crate) struct PassDesc {
    pub(crate) name: String,
    pub(crate) kind: PassKind,
}

/// pass 类型。
///
/// Transfer 变体由单测覆盖（数据流校验/环检测）；每帧实际图接入 transfer pass
/// 在后续批次（异步上传，M4）。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum PassKind {
    /// 图形 pass：color 附件（写）+ 可选 depth 附件（写）；动态渲染。
    Graphics {
        color: Vec<ResourceId>,
        depth: Option<ResourceId>,
    },
    /// 传输 pass：buffer 读/写声明（v0.5 每帧图不含 transfer pass，
    /// 网格上传保留在独立阻塞路径；类型先行以覆盖数据模型）。
    Transfer {
        reads: Vec<ResourceId>,
        writes: Vec<ResourceId>,
    },
}

/// 资源声明。
#[derive(Debug, Clone)]
pub(crate) struct ResourceDesc {
    pub(crate) name: String,
    pub(crate) kind: ResourceKind,
}

/// 资源类型。
///
/// TransientImage 由单测覆盖（别名槽推导）；实际接入 transient 资源在后续批次。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum ResourceKind {
    /// 每帧临时图像：图外负责按交换链 extent 创建/销毁，别名槽由编译器推导。
    TransientImage { format: ImageFormat },
    /// 外部导入图像：`initial` 为进入图时的布局，`final_layout` 为帧末要转到的布局。
    ImportedImage {
        initial: ImageLayout,
        final_layout: Option<ImageLayout>,
    },
    /// 外部导入 buffer（顶点/索引/UBO 等；无图像布局）。
    ImportedBuffer,
}

/// 图编译/校验失败。
#[derive(Debug, Clone, PartialEq, Error)]
pub(crate) enum GraphError {
    #[error("重复的资源名：{0}")]
    DuplicateResource(String),
    #[error("重复的 pass 名：{0}")]
    DuplicatePass(String),
    #[error("pass「{pass}」引用了未知资源（下标 {resource}）")]
    UnknownResource { pass: String, resource: usize },
    #[error("pass「{pass}」重复引用资源「{resource}」")]
    DuplicateUse { pass: String, resource: String },
    #[error("pass「{pass}」对资源「{resource}」的用法与资源类型不匹配")]
    MismatchedUsage { pass: String, resource: String },
    #[error("pass 依赖成环：{}", .0.join(" → "))]
    CycleDetected(Vec<String>),
}

/// 编译器生成的一条图像 barrier/layout 转换。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Barrier {
    pub(crate) resource: ResourceId,
    pub(crate) aspect: ImageAspect,
    pub(crate) src_stage: Stages,
    pub(crate) dst_stage: Stages,
    pub(crate) src_access: Access,
    pub(crate) dst_access: Access,
    pub(crate) src_layout: ImageLayout,
    pub(crate) dst_layout: ImageLayout,
}

/// 图编译产物：执行序 + 每个 pass 前置的 barrier + 帧末 barrier + 别名槽。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CompiledGraph {
    /// 拓扑排序后的 pass 执行序。
    pub(crate) pass_order: Vec<PassId>,
    /// 与 `pass_order` 对齐：各 pass 录制前要执行的 barrier。
    pub(crate) pre_pass_barriers: Vec<Vec<Barrier>>,
    /// 全部 pass 结束后的收尾 barrier（如交换链图像 → PRESENT）。
    pub(crate) final_barriers: Vec<Barrier>,
    /// 与图资源对齐：transient 图像的别名槽（`None` 表示非 transient 或未使用）。
    pub(crate) alias_slots: Vec<Option<usize>>,
}

/// 声明式渲染图：每帧由后端构建，`compile` 产生录制计划。
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderGraph {
    passes: Vec<PassDesc>,
    resources: Vec<ResourceDesc>,
}

impl RenderGraph {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 导入外部图像资源。
    pub(crate) fn import_image(
        &mut self,
        name: String,
        initial: ImageLayout,
        final_layout: Option<ImageLayout>,
    ) -> ResourceId {
        let id = ResourceId(self.resources.len());
        self.resources.push(ResourceDesc {
            name,
            kind: ResourceKind::ImportedImage {
                initial,
                final_layout,
            },
        });
        id
    }

    /// 导入外部 buffer 资源。
    pub(crate) fn import_buffer(&mut self, name: String) -> ResourceId {
        let id = ResourceId(self.resources.len());
        self.resources.push(ResourceDesc {
            name,
            kind: ResourceKind::ImportedBuffer,
        });
        id
    }

    /// 声明每帧临时图像（别名池的推导目标）。
    #[allow(dead_code)] // 仅单测使用；实际接入 transient 资源在后续批次。
    pub(crate) fn transient_image(&mut self, name: String, format: ImageFormat) -> ResourceId {
        let id = ResourceId(self.resources.len());
        self.resources.push(ResourceDesc {
            name,
            kind: ResourceKind::TransientImage { format },
        });
        id
    }

    /// 声明图形 pass。
    pub(crate) fn add_graphics_pass(
        &mut self,
        name: String,
        color: Vec<ResourceId>,
        depth: Option<ResourceId>,
    ) -> PassId {
        let id = PassId(self.passes.len());
        self.passes.push(PassDesc {
            name,
            kind: PassKind::Graphics { color, depth },
        });
        id
    }

    /// 声明传输 pass。
    #[allow(dead_code)] // 仅单测使用；每帧实际图接入 transfer pass 在后续批次。
    pub(crate) fn add_transfer_pass(
        &mut self,
        name: String,
        reads: Vec<ResourceId>,
        writes: Vec<ResourceId>,
    ) -> PassId {
        let id = PassId(self.passes.len());
        self.passes.push(PassDesc {
            name,
            kind: PassKind::Transfer { reads, writes },
        });
        id
    }

    pub(crate) fn passes(&self) -> &[PassDesc] {
        &self.passes
    }

    #[allow(dead_code)] // 预留给后端按名翻译资源（v0.5 通过 build_frame_graph 的映射表）。
    pub(crate) fn resources(&self) -> &[ResourceDesc] {
        &self.resources
    }

    /// 收集 pass 的资源访问（color 按声明序在前，depth 最后）。
    fn pass_usages(&self, pass_index: usize) -> Vec<(ResourceId, Usage)> {
        let mut usages = Vec::new();
        match &self.passes[pass_index].kind {
            PassKind::Graphics { color, depth } => {
                for resource in color {
                    usages.push((*resource, Usage::ColorWrite));
                }
                if let Some(resource) = depth {
                    usages.push((*resource, Usage::DepthWrite));
                }
            }
            PassKind::Transfer { reads, writes } => {
                for resource in reads {
                    usages.push((*resource, Usage::BufferRead));
                }
                for resource in writes {
                    usages.push((*resource, Usage::BufferWrite));
                }
            }
        }
        usages
    }

    /// 编译图：校验 → 数据流边 → 拓扑排序 → 布局追踪生成 barrier → 别名槽推导。
    pub(crate) fn compile(&self) -> Result<CompiledGraph, GraphError> {
        // 名字去重（R-03 验收项）。
        let mut seen_names = HashSet::new();
        for resource in &self.resources {
            if !seen_names.insert(resource.name.as_str()) {
                return Err(GraphError::DuplicateResource(resource.name.clone()));
            }
        }
        for pass in &self.passes {
            if !seen_names.insert(pass.name.as_str()) {
                return Err(GraphError::DuplicatePass(pass.name.clone()));
            }
        }

        // 收集各 pass 的资源访问并校验：未知资源、重复引用、类型匹配。
        let pass_usages: Vec<Vec<(ResourceId, Usage)>> = (0..self.passes.len())
            .map(|index| self.pass_usages(index))
            .collect();
        for (pass_index, uses) in pass_usages.iter().enumerate() {
            let pass_name = &self.passes[pass_index].name;
            // 图形 pass 的附件不允许重复引用；传输 pass 允许同资源读写（自洽）。
            if matches!(self.passes[pass_index].kind, PassKind::Graphics { .. }) {
                let mut used = HashSet::new();
                for (resource, _) in uses {
                    if !used.insert(resource.0) {
                        let resource_name = self.resources[resource.0].name.clone();
                        return Err(GraphError::DuplicateUse {
                            pass: pass_name.clone(),
                            resource: resource_name,
                        });
                    }
                }
            }
            for (resource, usage) in uses {
                let Some(desc) = self.resources.get(resource.0) else {
                    return Err(GraphError::UnknownResource {
                        pass: pass_name.clone(),
                        resource: resource.0,
                    });
                };
                let matched = matches!(
                    (&desc.kind, usage),
                    (
                        ResourceKind::ImportedImage { .. } | ResourceKind::TransientImage { .. },
                        Usage::ColorWrite | Usage::DepthWrite
                    ) | (
                        ResourceKind::ImportedBuffer,
                        Usage::BufferRead | Usage::BufferWrite
                    )
                );
                if !matched {
                    return Err(GraphError::MismatchedUsage {
                        pass: pass_name.clone(),
                        resource: desc.name.clone(),
                    });
                }
            }
        }

        // 数据流边（跳过同一 pass 内部的读写对）：
        // - 写→其后的读（RAW）/ 写（WAW，按声明序）
        // - 读→其后的写（WAR）
        // 同一资源上「先读后写」的跨 pass 对额外补一条写→读的 RAW 边，
        // 与 WAR 边构成环（读取早于其数据来源，数据流矛盾）。
        let mut per_resource: Vec<Vec<(usize, bool)>> = vec![Vec::new(); self.resources.len()];
        for (pass_index, uses) in pass_usages.iter().enumerate() {
            for (resource, usage) in uses {
                per_resource[resource.0].push((pass_index, usage.is_write()));
            }
        }
        let mut edges: Vec<HashSet<usize>> = vec![HashSet::new(); self.passes.len()];
        let mut indegree = vec![0usize; self.passes.len()];
        let add_edge =
            |from: usize, to: usize, edges: &mut Vec<HashSet<usize>>, indegree: &mut Vec<usize>| {
                if from != to && edges[from].insert(to) {
                    indegree[to] += 1;
                }
            };
        for uses in &per_resource {
            for (i, (a, a_write)) in uses.iter().enumerate() {
                for (b, b_write) in &uses[i + 1..] {
                    match (a_write, b_write) {
                        // 写→后用：RAW / WAW。
                        (true, _) => add_edge(*a, *b, &mut edges, &mut indegree),
                        // 读→后写：WAR；读取早于其数据来源 → 补反向 RAW 边构成环。
                        (false, true) => {
                            add_edge(*a, *b, &mut edges, &mut indegree);
                            add_edge(*b, *a, &mut edges, &mut indegree);
                        }
                        // 读→读：无需定序。
                        (false, false) => {}
                    }
                }
            }
        }

        // Kahn 拓扑排序（同层按声明序弹出）。
        let mut ready = BinaryHeap::new();
        for (index, &degree) in indegree.iter().enumerate() {
            if degree == 0 {
                ready.push(Reverse(index));
            }
        }
        let mut pass_order = Vec::with_capacity(self.passes.len());
        while let Some(Reverse(next)) = ready.pop() {
            pass_order.push(next);
            for (dependent, degree) in indegree.iter_mut().enumerate() {
                if edges[next].remove(&dependent) {
                    *degree -= 1;
                    if *degree == 0 {
                        ready.push(Reverse(dependent));
                    }
                }
            }
        }
        if pass_order.len() != self.passes.len() {
            let stuck: Vec<String> = (0..self.passes.len())
                .filter(|index| !pass_order.contains(index))
                .map(|index| self.passes[index].name.clone())
                .collect();
            return Err(GraphError::CycleDetected(stuck));
        }

        // 布局追踪：逐 pass 推导前置 barrier；帧末按 imported 资源的 final 布局收尾。
        let mut current_layout: Vec<ImageLayout> = self
            .resources
            .iter()
            .map(|desc| match &desc.kind {
                ResourceKind::TransientImage { .. } => ImageLayout::Undefined,
                ResourceKind::ImportedImage { initial, .. } => *initial,
                ResourceKind::ImportedBuffer => ImageLayout::Undefined,
            })
            .collect();
        let mut last_write: Vec<Option<(Stages, Access)>> = vec![None; self.resources.len()];
        let mut aspect_of: Vec<ImageAspect> = vec![ImageAspect::Color; self.resources.len()];
        let mut pre_pass_barriers = Vec::with_capacity(pass_order.len());
        for &pass_index in &pass_order {
            let mut barriers = Vec::new();
            for (resource, usage) in &pass_usages[pass_index] {
                let Some(required) = usage.layout() else {
                    continue; // buffer 无布局。
                };
                if current_layout[resource.0] != required {
                    let (src_stage, src_access) =
                        last_write[resource.0].unwrap_or((Stages::TOP_OF_PIPE, Access::NONE));
                    let (dst_stage, dst_access) = usage.stage_access();
                    barriers.push(Barrier {
                        resource: *resource,
                        aspect: aspect_of[resource.0],
                        src_stage,
                        dst_stage,
                        src_access,
                        dst_access,
                        src_layout: current_layout[resource.0],
                        dst_layout: required,
                    });
                }
                current_layout[resource.0] = required;
                let (stage, access) = usage.stage_access();
                last_write[resource.0] = Some((stage, access));
                aspect_of[resource.0] = if matches!(usage, Usage::DepthWrite) {
                    ImageAspect::Depth
                } else {
                    ImageAspect::Color
                };
            }
            pre_pass_barriers.push(barriers);
        }

        let mut final_barriers = Vec::new();
        for (index, desc) in self.resources.iter().enumerate() {
            let ResourceKind::ImportedImage {
                final_layout: Some(final_layout),
                ..
            } = &desc.kind
            else {
                continue;
            };
            if current_layout[index] == *final_layout {
                continue;
            }
            let (src_stage, src_access) =
                last_write[index].unwrap_or((Stages::TOP_OF_PIPE, Access::NONE));
            final_barriers.push(Barrier {
                resource: ResourceId(index),
                aspect: aspect_of[index],
                src_stage,
                dst_stage: Stages::BOTTOM_OF_PIPE,
                src_access,
                dst_access: Access::NONE,
                src_layout: current_layout[index],
                dst_layout: *final_layout,
            });
        }

        // 别名槽（R-05）：transient 图像按拓扑序计算生命周期，贪心复用不重叠的槽。
        let mut alias_slots: Vec<Option<usize>> = vec![None; self.resources.len()];
        let mut lifetimes: Vec<(usize, usize, usize)> = Vec::new(); // (resource, first, last)
        for (index, desc) in self.resources.iter().enumerate() {
            if !matches!(desc.kind, ResourceKind::TransientImage { .. }) {
                continue;
            }
            let positions: Vec<usize> = pass_order
                .iter()
                .filter(|&&pass_index| {
                    pass_usages[pass_index]
                        .iter()
                        .any(|(resource, _)| resource.0 == index)
                })
                .copied()
                .collect();
            if let (Some(&first), Some(&last)) = (positions.first(), positions.last()) {
                lifetimes.push((index, first, last));
            }
        }
        lifetimes.sort_by_key(|&(_, first, _)| first);
        let mut slots: Vec<usize> = Vec::new(); // 各槽位当前占用者的生命周期终点
        for &(resource, first, last) in &lifetimes {
            let slot = slots
                .iter()
                .position(|&end| end < first)
                .unwrap_or_else(|| {
                    slots.push(0);
                    slots.len() - 1
                });
            slots[slot] = last;
            alias_slots[resource] = Some(slot);
        }

        Ok(CompiledGraph {
            pass_order: pass_order.into_iter().map(PassId).collect(),
            pre_pass_barriers,
            final_barriers,
            alias_slots,
        })
    }

    /// 依赖图 dump（R-05）：DOT 文本，资源节点 + pass 边 + barrier 标注 + 别名注释。
    pub(crate) fn dump_dot(&self, compiled: &CompiledGraph) -> String {
        let mut out = String::from("digraph RenderGraph {\n    rankdir=LR;\n\n");
        for (index, resource) in self.resources.iter().enumerate() {
            let kind = match &resource.kind {
                ResourceKind::TransientImage { format } => {
                    format!("transient image ({format})")
                }
                ResourceKind::ImportedImage {
                    initial,
                    final_layout,
                } => match final_layout {
                    Some(final_layout) => format!(
                        "imported image ({} → {})",
                        initial.label(),
                        final_layout.label()
                    ),
                    None => format!("imported image ({}, 静态)", initial.label()),
                },
                ResourceKind::ImportedBuffer => "imported buffer".to_string(),
            };
            let name = &resource.name;
            out.push_str(&format!(
                "    r{index} [label=\"{name}\\n{kind}\", shape=ellipse];\n"
            ));
        }
        out.push('\n');
        for (index, pass) in self.passes.iter().enumerate() {
            let kind = match &pass.kind {
                PassKind::Graphics { .. } => "graphics",
                PassKind::Transfer { .. } => "transfer",
            };
            let name = &pass.name;
            out.push_str(&format!(
                "    p{index} [label=\"{name}\\n{kind}\", shape=box];\n"
            ));
        }
        out.push('\n');
        for (pass_index, _) in self.passes.iter().enumerate() {
            for (resource, usage) in self.pass_usages(pass_index) {
                if usage.is_write() {
                    out.push_str(&format!(
                        "    p{pass_index} -> r{} [label=\"{}\"];\n",
                        resource.0,
                        usage.label()
                    ));
                } else {
                    out.push_str(&format!(
                        "    r{} -> p{pass_index} [label=\"{}\"];\n",
                        resource.0,
                        usage.label()
                    ));
                }
            }
        }
        out.push('\n');
        for (position, barriers) in compiled.pre_pass_barriers.iter().enumerate() {
            let pass_index = compiled.pass_order[position].0;
            for barrier in barriers {
                out.push_str(&format!(
                    "    p{pass_index} -> r{} [style=dashed, constraint=false, \
                     label=\"barrier {} → {}\\n{} | {}\"];\n",
                    barrier.resource.0,
                    barrier.src_layout.label(),
                    barrier.dst_layout.label(),
                    barrier.src_stage.label(),
                    barrier.dst_stage.label(),
                ));
            }
        }
        if !compiled.final_barriers.is_empty() {
            out.push_str("    frame_end [label=\"frame_end\", shape=box, style=rounded];\n");
            for barrier in &compiled.final_barriers {
                out.push_str(&format!(
                    "    r{} -> frame_end [style=dashed, constraint=false, \
                     label=\"barrier {} → {}\\n{} | {}\"];\n",
                    barrier.resource.0,
                    barrier.src_layout.label(),
                    barrier.dst_layout.label(),
                    barrier.src_stage.label(),
                    barrier.dst_stage.label(),
                ));
            }
        }
        let aliases: Vec<String> = compiled
            .alias_slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.map(|slot| format!("r{index}@slot{slot}")))
            .collect();
        if !aliases.is_empty() {
            out.push_str(&format!("    // alias: {}\n", aliases.join(", ")));
        }
        out.push_str("}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小可运行图：imported 交换链（UNDEFINED → PRESENT）+ 深度（静态布局）+ 单 graphics pass。
    fn minimal_graph() -> (RenderGraph, ResourceId, ResourceId) {
        let mut graph = RenderGraph::new();
        let swapchain = graph.import_image(
            "swapchain".to_string(),
            ImageLayout::Undefined,
            Some(ImageLayout::Present),
        );
        let depth = graph.import_image("depth".to_string(), ImageLayout::DepthAttachment, None);
        graph.add_graphics_pass("main_geometry".to_string(), vec![swapchain], Some(depth));
        (graph, swapchain, depth)
    }

    #[test]
    fn minimal_graph_generates_acquire_and_present_barriers() {
        let (graph, swapchain, depth) = minimal_graph();
        let compiled = graph.compile().expect("最小图应可编译");
        assert_eq!(compiled.pass_order.len(), 1);
        assert_eq!(compiled.pass_order[0], PassId(0));

        // acquire 后：UNDEFINED → COLOR_ATTACHMENT（颜色写）。
        let pre = &compiled.pre_pass_barriers[0];
        assert_eq!(
            pre.len(),
            1,
            "深度已处于 DEPTH_ATTACHMENT，不应生成 barrier"
        );
        let barrier = &pre[0];
        assert_eq!(barrier.resource, swapchain);
        assert_eq!(barrier.src_layout, ImageLayout::Undefined);
        assert_eq!(barrier.dst_layout, ImageLayout::ColorAttachment);
        assert!(barrier.dst_stage.contains(Stages::COLOR_ATTACHMENT_OUTPUT));
        assert!(barrier.dst_access.contains(Access::COLOR_ATTACHMENT_WRITE));

        // 帧末：COLOR_ATTACHMENT → PRESENT。
        assert_eq!(compiled.final_barriers.len(), 1);
        let barrier = &compiled.final_barriers[0];
        assert_eq!(barrier.resource, swapchain);
        assert_eq!(barrier.src_layout, ImageLayout::ColorAttachment);
        assert_eq!(barrier.dst_layout, ImageLayout::Present);
        assert!(barrier.src_stage.contains(Stages::COLOR_ATTACHMENT_OUTPUT));
        assert!(barrier.src_access.contains(Access::COLOR_ATTACHMENT_WRITE));
        assert!(barrier.dst_stage.contains(Stages::BOTTOM_OF_PIPE));

        // 深度图像：初始即 DEPTH_ATTACHMENT，pass 使用中无 barrier、帧末无收尾。
        assert!(
            compiled
                .pre_pass_barriers
                .iter()
                .flatten()
                .all(|barrier| barrier.resource != depth)
        );
        assert!(compiled.final_barriers.iter().all(|b| b.resource != depth));
    }

    #[test]
    fn duplicate_resource_and_pass_names_are_rejected() {
        let mut graph = RenderGraph::new();
        graph.import_image("a".to_string(), ImageLayout::Undefined, None);
        graph.import_image("a".to_string(), ImageLayout::Undefined, None);
        assert!(matches!(
            graph.compile(),
            Err(GraphError::DuplicateResource(_))
        ));

        let mut graph = RenderGraph::new();
        let resource = graph.import_image("a".to_string(), ImageLayout::Undefined, None);
        graph.add_graphics_pass("p".to_string(), vec![resource], None);
        graph.add_graphics_pass("p".to_string(), vec![resource], None);
        assert!(matches!(graph.compile(), Err(GraphError::DuplicatePass(_))));
    }

    #[test]
    fn unknown_resource_is_rejected() {
        let mut graph = RenderGraph::new();
        graph.add_graphics_pass("p".to_string(), vec![ResourceId(7)], None);
        assert!(matches!(
            graph.compile(),
            Err(GraphError::UnknownResource { resource: 7, .. })
        ));
    }

    #[test]
    fn graphics_pass_duplicate_attachment_is_rejected() {
        let mut graph = RenderGraph::new();
        let resource = graph.import_image("a".to_string(), ImageLayout::Undefined, None);
        graph.add_graphics_pass("p".to_string(), vec![resource, resource], None);
        assert!(matches!(
            graph.compile(),
            Err(GraphError::DuplicateUse { .. })
        ));
    }

    #[test]
    fn usage_kind_mismatch_is_rejected() {
        let mut graph = RenderGraph::new();
        let buffer = graph.import_buffer("buf".to_string());
        graph.add_graphics_pass("p".to_string(), vec![buffer], None);
        assert!(matches!(
            graph.compile(),
            Err(GraphError::MismatchedUsage { .. })
        ));
    }

    #[test]
    fn transfer_write_then_read_keeps_declaration_order() {
        let mut graph = RenderGraph::new();
        let buffer = graph.import_buffer("buf".to_string());
        let writer = graph.add_transfer_pass("upload".to_string(), vec![], vec![buffer]);
        let reader = graph.add_transfer_pass("consume".to_string(), vec![buffer], vec![]);
        let compiled = graph.compile().expect("写后读应可编译");
        assert_eq!(compiled.pass_order, vec![writer, reader]);
        // buffer 无布局：不生成 barrier。
        assert!(
            compiled
                .pre_pass_barriers
                .iter()
                .all(|barriers| barriers.is_empty())
        );
    }

    #[test]
    fn read_before_write_is_rejected_as_cycle() {
        let mut graph = RenderGraph::new();
        let buffer = graph.import_buffer("buf".to_string());
        // 声明序「先读后写」：读取早于其数据来源，数据流矛盾 → 环。
        graph.add_transfer_pass("consume".to_string(), vec![buffer], vec![]);
        graph.add_transfer_pass("upload".to_string(), vec![], vec![buffer]);
        assert!(matches!(graph.compile(), Err(GraphError::CycleDetected(_))));
    }

    #[test]
    fn mutual_dataflow_cycle_is_rejected() {
        let mut graph = RenderGraph::new();
        let r1 = graph.import_buffer("r1".to_string());
        let r2 = graph.import_buffer("r2".to_string());
        // A 读 r1 写 r2；B 写 r1 读 r2：互为输入输出 → 环。
        graph.add_transfer_pass("A".to_string(), vec![r1], vec![r2]);
        graph.add_transfer_pass("B".to_string(), vec![r2], vec![r1]);
        let error = graph.compile().expect_err("互为输入输出应判定为环");
        let GraphError::CycleDetected(names) = error else {
            panic!("应为 CycleDetected，实际：{error:?}");
        };
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn disjoint_transient_lifetimes_share_alias_slot() {
        let mut graph = RenderGraph::new();
        let a = graph.transient_image("a".to_string(), ImageFormat::R8G8B8A8Unorm);
        let b = graph.transient_image("b".to_string(), ImageFormat::D32Sfloat);
        graph.add_graphics_pass("write_a".to_string(), vec![a], None);
        graph.add_graphics_pass("write_b".to_string(), vec![b], None);
        let compiled = graph.compile().expect("disjoint 生命周期应可编译");
        assert_eq!(compiled.alias_slots[a.0], Some(0));
        assert_eq!(
            compiled.alias_slots[b.0],
            Some(0),
            "生命周期不重叠应复用槽位"
        );
    }

    #[test]
    fn overlapping_transient_lifetimes_get_distinct_slots() {
        let mut graph = RenderGraph::new();
        let a = graph.transient_image("a".to_string(), ImageFormat::R8G8B8A8Unorm);
        let b = graph.transient_image("b".to_string(), ImageFormat::R8G8B8A8Unorm);
        graph.add_graphics_pass("write_a".to_string(), vec![a], None);
        graph.add_graphics_pass("mixed".to_string(), vec![a, b], None);
        let compiled = graph.compile().expect("重叠生命周期应可编译");
        let slot_a = compiled.alias_slots[a.0];
        let slot_b = compiled.alias_slots[b.0];
        assert_ne!(slot_a, slot_b, "生命周期重叠不得复用槽位");
    }

    #[test]
    fn empty_graph_compiles_to_empty_plan() {
        let graph = RenderGraph::new();
        let compiled = graph.compile().expect("空图应可编译");
        assert!(compiled.pass_order.is_empty());
        assert!(compiled.pre_pass_barriers.is_empty());
        assert!(compiled.final_barriers.is_empty());
    }

    #[test]
    fn dot_dump_contains_resources_passes_and_barriers() {
        let (graph, _, _) = minimal_graph();
        let compiled = graph.compile().expect("最小图应可编译");
        let dot = graph.dump_dot(&compiled);
        assert!(dot.starts_with("digraph RenderGraph"));
        assert!(dot.contains("swapchain"));
        assert!(dot.contains("main_geometry"));
        assert!(dot.contains("barrier UNDEFINED → COLOR_ATTACHMENT"));
        assert!(dot.contains("barrier COLOR_ATTACHMENT → PRESENT"));
        assert!(dot.contains("COLOR_ATTACHMENT_OUTPUT"));
    }

    #[test]
    fn stage_and_access_labels_are_readable() {
        let stages = Stages::EARLY_FRAGMENT_TESTS.union(Stages::LATE_FRAGMENT_TESTS);
        assert_eq!(stages.label(), "EARLY_FRAGMENT_TESTS|LATE_FRAGMENT_TESTS");
        assert_eq!(Stages::NONE.label(), "NONE");
        assert_eq!(Access::NONE.label(), "NONE");
    }
}
