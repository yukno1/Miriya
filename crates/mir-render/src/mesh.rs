//! 网格数据与句柄。
//!
//! 渲染层只接受纯数据（顶点/索引），不访问 ECS/场景（ADR-0004 的 extract 边界）。

/// 顶点：位置 + 颜色 + 法线（R-12：stride 24 → 36）。
///
/// `repr(C)` 且只含 `f32`，因此没有填充、可按字节上传（见 [`vertices_as_bytes`]）。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    /// 模型空间位置。
    pub position: [f32; 3],
    /// 线性颜色（RGBA 中的 RGB，alpha 由管线固定为 1）。
    pub color: [f32; 3],
    /// 模型空间法线（假定单位长度；PBR 光照用）。
    pub normal: [f32; 3],
}

impl Vertex {
    /// 单个顶点的字节数（9 × f32，无填充）。
    pub const SIZE: usize = 36;

    pub const fn new(position: [f32; 3], color: [f32; 3], normal: [f32; 3]) -> Self {
        Self {
            position,
            color,
            normal,
        }
    }
}

/// 顶点切片 → 字节视图（用于上传到 GPU）。
pub(crate) fn vertices_as_bytes(vertices: &[Vertex]) -> &[u8] {
    // SAFETY: `Vertex` 为 repr(C) 且只含 f32，无填充、大小 = 36；
    // u8 对齐要求更低，按字节读取整个切片是合法的。
    unsafe {
        std::slice::from_raw_parts(
            vertices.as_ptr().cast::<u8>(),
            std::mem::size_of_val(vertices),
        )
    }
}

/// 索引切片 → 字节视图。
pub(crate) fn indices_as_bytes(indices: &[u32]) -> &[u8] {
    // SAFETY: u32 无填充，按 u8 读取整个切片合法。
    unsafe {
        std::slice::from_raw_parts(
            indices.as_ptr().cast::<u8>(),
            std::mem::size_of_val(indices),
        )
    }
}

/// CPU 侧网格数据：创建 GPU 网格时一次性上传。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self { vertices, indices }
    }

    /// 是否可用于绘制：顶点非空、索引非空且全部指向已存在的顶点。
    pub fn is_valid(&self) -> bool {
        !self.vertices.is_empty()
            && !self.indices.is_empty()
            && self
                .indices
                .iter()
                .all(|index| (*index as usize) < self.vertices.len())
    }
}

/// 网格句柄：不透明、可复制；内部编码槽位与世代号，销毁后旧句柄自动失效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshHandle(u64);

impl MeshHandle {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_layout_is_36_bytes() {
        assert_eq!(std::mem::size_of::<Vertex>(), Vertex::SIZE);
        assert_eq!(std::mem::align_of::<Vertex>(), 4);

        let vertex = Vertex::new([1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]);
        let bytes = vertices_as_bytes(std::slice::from_ref(&vertex));
        assert_eq!(bytes.len(), Vertex::SIZE);
        assert_eq!(&bytes[0..4], &1.0f32.to_ne_bytes());
        assert_eq!(&bytes[12..16], &4.0f32.to_ne_bytes());
        assert_eq!(&bytes[24..28], &7.0f32.to_ne_bytes());
    }

    #[test]
    fn index_bytes_match_len() {
        let indices = [0u32, 1, 2];
        assert_eq!(indices_as_bytes(&indices).len(), 12);
    }

    #[test]
    fn mesh_validity_rejects_edges() {
        let empty = MeshData::default();
        assert!(!empty.is_valid());

        let no_indices = MeshData::new(vec![Vertex::new([0.0; 3], [0.0; 3], [0.0; 3])], Vec::new());
        assert!(!no_indices.is_valid());

        let out_of_range =
            MeshData::new(vec![Vertex::new([0.0; 3], [0.0; 3], [0.0; 3])], vec![0, 1]);
        assert!(!out_of_range.is_valid());

        let valid = MeshData::new(
            vec![
                Vertex::new([0.0; 3], [0.0; 3], [0.0; 3]),
                Vertex::new([1.0; 3], [1.0; 3], [0.0; 3]),
                Vertex::new([2.0; 3], [2.0; 3], [0.0; 3]),
            ],
            vec![0, 1, 2],
        );
        assert!(valid.is_valid());
    }

    #[test]
    fn handle_encodes_index_and_generation() {
        let handle = MeshHandle::from_parts(7, 3);
        assert_eq!(handle.index(), 7);
        assert_eq!(handle.generation(), 3);
        assert_ne!(handle, MeshHandle::from_parts(7, 4));
        assert_ne!(handle, MeshHandle::from_parts(8, 3));
    }
}
