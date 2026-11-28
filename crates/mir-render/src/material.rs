//! 材质数据与句柄（R-11）。
//!
//! v0.3 边界（m2-remaining.md 决策 6）：材质 = 参数 UBO（metallic-roughness）+
//! 纹理 binding 占位（M5 descriptor indexing 时再数组化）；`repr(C)` 且只含 `f32`，
//! 字段偏移与 GLSL std140 布局一一对应（见 [`MaterialData`] 字段文档）。
//!
//! 渲染层只接受纯数据，不访问 ECS/场景（ADR-0012 extract 边界）。

/// 材质槽位上限（两后端共享；同时决定 Vulkan 描述符池容量）。
pub(crate) const MAX_MATERIALS: usize = 64;

/// 材质参数（PBR metallic-roughness，R-11）。
///
/// std140 布局对应：`vec4 base_color`（align 16）→ `float metallic` → `float roughness`
/// → `vec3 emissive`（align 16，因此补齐到偏移 32）→ 总大小 48 字节。
/// `_pad0` / `_pad1` 是 Rust 侧对齐占位，`repr(C)` 且全部字段为 `f32`，无隐式填充。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialData {
    /// 基础反照率（RGB）+ alpha 占位（线性空间）。
    pub base_color: [f32; 4],
    /// 金属度 0 = 电介质，1 = 金属。
    pub metallic: f32,
    /// 粗糙度 0 = 镜面，1 = 完全粗糙。
    pub roughness: f32,
    /// std140 对齐占位（偏移 24…31）。
    _pad0: [f32; 2],
    /// 自发光（RGB，线性空间；v0.3 直接加到输出色）。
    pub emissive: [f32; 3],
    /// 补齐到 16 字节倍数（偏移 44…47）。
    _pad1: f32,
}

impl MaterialData {
    /// 单个材质 UBO 的字节数（std140：`vec4` + 2×`float` + `vec3` 对齐补齐）。
    pub const SIZE: usize = 48;

    pub const fn new(
        base_color: [f32; 4],
        metallic: f32,
        roughness: f32,
        emissive: [f32; 3],
    ) -> Self {
        Self {
            base_color,
            metallic,
            roughness,
            _pad0: [0.0; 2],
            emissive,
            _pad1: 0.0,
        }
    }
}

impl Default for MaterialData {
    /// 白色 Lambert 基准（metallic 0 / roughness 1）——渲染层的默认材质语义。
    fn default() -> Self {
        Self::new([1.0, 1.0, 1.0, 1.0], 0.0, 1.0, [0.0; 3])
    }
}

/// 材质句柄：不透明、可复制；内部编码槽位与世代号，销毁后旧句柄自动失效
/// （风格对齐 [`crate::MeshHandle`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialHandle(u64);

impl MaterialHandle {
    pub(crate) fn from_parts(index: u32, generation: u32) -> Self {
        Self(((generation as u64) << 32) | index as u64)
    }

    pub(crate) fn index(self) -> usize {
        (self.0 & 0xFFFF_FFFF) as usize
    }

    pub(crate) fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

/// 材质参数 → 字节视图（上传到材质 UBO）。
pub(crate) fn material_data_bytes(data: &MaterialData) -> &[u8] {
    // SAFETY: `MaterialData` 为 repr(C) 且只含 f32（含占位字段），无隐式填充、
    // 大小 = 48；u8 对齐要求更低，按字节读取是合法的。
    unsafe {
        std::slice::from_raw_parts(
            (data as *const MaterialData).cast::<u8>(),
            std::mem::size_of::<MaterialData>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_layout_is_48_bytes_and_matches_std140() {
        assert_eq!(std::mem::size_of::<MaterialData>(), MaterialData::SIZE);
        assert_eq!(std::mem::align_of::<MaterialData>(), 4);

        assert_eq!(std::mem::offset_of!(MaterialData, base_color), 0);
        assert_eq!(std::mem::offset_of!(MaterialData, metallic), 16);
        assert_eq!(std::mem::offset_of!(MaterialData, roughness), 20);
        assert_eq!(std::mem::offset_of!(MaterialData, emissive), 32);

        let data = MaterialData::new([0.1, 0.2, 0.3, 0.4], 0.5, 0.6, [0.7, 0.8, 0.9]);
        let bytes = material_data_bytes(&data);
        assert_eq!(bytes.len(), MaterialData::SIZE);
        assert_eq!(&bytes[0..4], &0.1f32.to_ne_bytes());
        assert_eq!(&bytes[16..20], &0.5f32.to_ne_bytes());
        assert_eq!(&bytes[20..24], &0.6f32.to_ne_bytes());
        assert_eq!(&bytes[32..36], &0.7f32.to_ne_bytes());
    }

    #[test]
    fn default_material_is_white_lambert() {
        let data = MaterialData::default();
        assert_eq!(data.base_color, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(data.metallic, 0.0);
        assert_eq!(data.roughness, 1.0);
        assert_eq!(data.emissive, [0.0; 3]);
    }

    #[test]
    fn handle_encodes_index_and_generation() {
        let handle = MaterialHandle::from_parts(3, 9);
        assert_eq!(handle.index(), 3);
        assert_eq!(handle.generation(), 9);
        assert_ne!(handle, MaterialHandle::from_parts(3, 10));
        assert_ne!(handle, MaterialHandle::from_parts(4, 9));
    }
}
