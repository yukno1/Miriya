//! 内置几何体。
//!
//! 只产出 [`MeshData`]；GPU 资源由渲染上下文在 `create_mesh` 时创建。

use crate::mesh::{MeshData, Vertex};

/// 每个面的 4 个顶点（从外部看为逆时针）与面颜色。
type Face = ([[f32; 3]; 4], [f32; 3]);

/// 六个面：+X / -X / +Y / -Y / +Z / -Z（ADR-0003：X 前、Y 右、Z 上）。
const FACES: [Face; 6] = [
    // +X（前，红）
    (
        [
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [0.5, 0.5, 0.5],
            [0.5, -0.5, 0.5],
        ],
        [1.0, 0.0, 0.0],
    ),
    // -X（后，青）
    (
        [
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ],
        [0.0, 1.0, 1.0],
    ),
    // +Y（右，绿）
    (
        [
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
        ],
        [0.0, 1.0, 0.0],
    ),
    // -Y（左，品红）
    (
        [
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, -0.5, 0.5],
            [-0.5, -0.5, 0.5],
        ],
        [1.0, 0.0, 1.0],
    ),
    // +Z（上，蓝）
    (
        [
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ],
        [0.0, 0.0, 1.0],
    ),
    // -Z（下，黄）
    (
        [
            [-0.5, 0.5, -0.5],
            [0.5, 0.5, -0.5],
            [0.5, -0.5, -0.5],
            [-0.5, -0.5, -0.5],
        ],
        [1.0, 1.0, 0.0],
    ),
];

/// 单位立方体：边长 1、中心在原点、每面独立顶点与颜色，逆时针绕序（从外部看）。
///
/// 24 顶点 / 36 索引；同一位置在不同面上各有一份顶点，便于逐面颜色与后续法线。
pub fn cube() -> MeshData {
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (face, color) in FACES {
        let base = vertices.len() as u32;
        for position in face {
            vertices.push(Vertex::new(position, color));
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    MeshData::new(vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn cube_has_expected_counts_and_bounds() {
        let mesh = cube();
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
        assert!(mesh.is_valid());
        assert!(mesh.indices.iter().all(|index| (*index as usize) < 24));

        // 每个面 4 个顶点、6 个索引，且共享 6 种颜色。
        let mut colors = Vec::new();
        for chunk in mesh.vertices.chunks(4) {
            let color = chunk[0].color;
            assert!(chunk.iter().all(|vertex| vertex.color == color));
            colors.push(color);
        }
        assert_eq!(colors.len(), 6);
    }

    #[test]
    fn cube_faces_wind_counter_clockwise_from_outside() {
        let mesh = cube();
        for face in 0..6 {
            let base = face * 4;
            let v0 = mesh.vertices[base].position;
            let v1 = mesh.vertices[base + 1].position;
            let v2 = mesh.vertices[base + 2].position;
            let normal = cross(sub(v1, v0), sub(v2, v0));

            // 面中心即向外的方向（立方体中心在原点）。
            let center = [
                (v0[0] + v1[0] + v2[0] + mesh.vertices[base + 3].position[0]) / 4.0,
                (v0[1] + v1[1] + v2[1] + mesh.vertices[base + 3].position[1]) / 4.0,
                (v0[2] + v1[2] + v2[2] + mesh.vertices[base + 3].position[2]) / 4.0,
            ];

            assert!(
                dot(normal, center) > 0.0,
                "第 {face} 个面的绕序与向外法线相反: normal={normal:?} center={center:?}"
            );
        }
    }

    #[test]
    fn cube_covers_all_six_face_directions() {
        let mesh = cube();
        let mut directions: Vec<[i32; 3]> = Vec::new();
        for face in 0..6 {
            let base = face * 4;
            let center = [
                (mesh.vertices[base].position[0]
                    + mesh.vertices[base + 1].position[0]
                    + mesh.vertices[base + 2].position[0]
                    + mesh.vertices[base + 3].position[0])
                    / 4.0,
                (mesh.vertices[base].position[1]
                    + mesh.vertices[base + 1].position[1]
                    + mesh.vertices[base + 2].position[1]
                    + mesh.vertices[base + 3].position[1])
                    / 4.0,
                (mesh.vertices[base].position[2]
                    + mesh.vertices[base + 1].position[2]
                    + mesh.vertices[base + 2].position[2]
                    + mesh.vertices[base + 3].position[2])
                    / 4.0,
            ];
            // 面中心落在某一坐标轴上；按符号折叠成 6 个互不相同的单位方向。
            let direction = [
                if center[0] > 0.4 {
                    1
                } else if center[0] < -0.4 {
                    -1
                } else {
                    0
                },
                if center[1] > 0.4 {
                    1
                } else if center[1] < -0.4 {
                    -1
                } else {
                    0
                },
                if center[2] > 0.4 {
                    1
                } else if center[2] < -0.4 {
                    -1
                } else {
                    0
                },
            ];
            assert_eq!(
                direction
                    .iter()
                    .filter(|component| **component != 0)
                    .count(),
                1,
                "面中心应只落在一个轴上: {direction:?}"
            );
            assert!(
                !directions.contains(&direction),
                "面方向重复: {direction:?}"
            );
            directions.push(direction);
        }
        assert_eq!(directions.len(), 6);
    }
}
