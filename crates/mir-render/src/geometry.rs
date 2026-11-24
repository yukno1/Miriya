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

/// 六个面的向外法线（与 [`FACES`] 顺序一致，R-12 起顶点带法线）。
const FACE_NORMALS: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
];

/// 单位立方体：边长 1、中心在原点、每面独立顶点与颜色，逆时针绕序（从外部看）。
///
/// 24 顶点 / 36 索引；同一位置在不同面上各有一份顶点，便于逐面颜色与逐面法线。
pub fn cube() -> MeshData {
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for ((face, color), normal) in FACES.iter().zip(FACE_NORMALS.iter()) {
        let base = vertices.len() as u32;
        for position in face {
            vertices.push(Vertex::new(*position, *color, *normal));
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    MeshData::new(vertices, indices)
}

/// 球体：中心在原点、半径 `radius`，Z 轴为极轴（经度 `slices` × 纬度 `stacks`）。
///
/// 每顶点法线 = 归一化位置；绕序从外部看为逆时针。极点行的退化三角形被跳过。
pub fn sphere(radius: f32, slices: u32, stacks: u32) -> MeshData {
    assert!(slices >= 3 && stacks >= 2, "球体分段数过少");
    let color = [0.75, 0.75, 0.78];
    let mut vertices = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for v in 0..=stacks {
        let phi = std::f32::consts::PI * v as f32 / stacks as f32;
        for u in 0..=slices {
            let theta = std::f32::consts::TAU * u as f32 / slices as f32;
            let (sin_theta, cos_theta) = theta.sin_cos();
            let (sin_phi, cos_phi) = phi.sin_cos();
            let position = [
                radius * sin_phi * cos_theta,
                radius * sin_phi * sin_theta,
                radius * cos_phi,
            ];
            let normal = [sin_phi * cos_theta, sin_phi * sin_theta, cos_phi];
            vertices.push(Vertex::new(position, color, normal));
        }
    }

    let stride = slices + 1;
    let mut indices = Vec::new();
    for v in 0..stacks {
        for u in 0..slices {
            let a = v * stride + u;
            let b = v * stride + u + 1;
            let c = (v + 1) * stride + u;
            let d = (v + 1) * stride + u + 1;
            // 两枚三角形共享对角线 a→d；绕序保证从外部看为逆时针（见单测）。
            // 极点行（v=0 或 v+1=stacks）中退化的一侧跳过。
            if v > 0 {
                indices.extend_from_slice(&[a, d, b]);
            }
            if v + 1 < stacks {
                indices.extend_from_slice(&[a, c, d]);
            }
        }
    }
    MeshData::new(vertices, indices)
}

/// 圆环体：主圆位于 XY 平面（Z 轴为对称轴）、中心在原点、主半径 `major`、管半径 `minor`。
///
/// 每顶点法线指向管外侧；绕序从外部看为逆时针。经缝与纬缝均通过取模索引闭合（无重复顶点）。
pub fn torus(major: f32, minor: f32, segments: u32, sides: u32) -> MeshData {
    assert!(segments >= 3 && sides >= 3, "圆环分段数过少");
    let color = [0.7, 0.72, 0.76];
    let mut vertices = Vec::with_capacity((segments * sides) as usize);
    for i in 0..segments {
        let u = std::f32::consts::TAU * i as f32 / segments as f32;
        let (cos_u, sin_u) = u.sin_cos();
        for j in 0..sides {
            let v = std::f32::consts::TAU * j as f32 / sides as f32;
            let (cos_v, sin_v) = v.sin_cos();
            let ring = major + minor * cos_v;
            let position = [ring * cos_u, ring * sin_u, minor * sin_v];
            let normal = [cos_v * cos_u, cos_v * sin_u, sin_v];
            vertices.push(Vertex::new(position, color, normal));
        }
    }

    let mut indices = Vec::new();
    for i in 0..segments {
        let i_next = (i + 1) % segments;
        for j in 0..sides {
            let j_next = (j + 1) % sides;
            let a = i * sides + j;
            let b = i * sides + j_next;
            let c = i_next * sides + j;
            let d = i_next * sides + j_next;
            indices.extend_from_slice(&[a, d, b, a, c, d]);
        }
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

    /// 逐三角形断言绕序与给定表面法线一致（叉积与法线同向）。
    /// 回调收到三角形首顶点位置与重心位置，返回该处的期望法线。
    fn assert_winding_outward(mesh: &MeshData, outward: impl Fn([f32; 3], [f32; 3]) -> [f32; 3]) {
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let v0 = mesh.vertices[triangle[0] as usize];
            let v1 = mesh.vertices[triangle[1] as usize];
            let v2 = mesh.vertices[triangle[2] as usize];
            let cross = cross(sub(v1.position, v0.position), sub(v2.position, v0.position));
            let center = [
                (v0.position[0] + v1.position[0] + v2.position[0]) / 3.0,
                (v0.position[1] + v1.position[1] + v2.position[1]) / 3.0,
                (v0.position[2] + v1.position[2] + v2.position[2]) / 3.0,
            ];
            assert!(
                dot(cross, outward(v0.position, center)) > 0.0,
                "三角形绕序与外向法线相反: a={:?} b={:?} c={:?}",
                v0.position,
                v1.position,
                v2.position
            );
        }
    }

    #[test]
    fn sphere_has_valid_normals_and_outward_winding() {
        let mesh = sphere(1.5, 24, 16);
        assert!(mesh.is_valid());
        // 极点行退化三角形已跳过：索引数 = (cells - 两行极点带) × 6，下限断言即可。
        assert!(mesh.indices.len() >= 6 * 24 * 14);

        // 法线 = 归一化位置，且为单位长度。
        for vertex in &mesh.vertices {
            let length = dot(vertex.normal, vertex.normal).sqrt();
            assert!((length - 1.0).abs() < 1e-4, "球体法线非单位长度: {length}");
            let expected = [
                vertex.position[0] / 1.5,
                vertex.position[1] / 1.5,
                vertex.position[2] / 1.5,
            ];
            assert!((dot(vertex.normal, expected) - 1.0).abs() < 1e-4);
        }

        assert_winding_outward(&mesh, |_, center| center);
    }

    #[test]
    fn torus_has_valid_normals_and_outward_winding() {
        let major = 1.2;
        let minor = 0.4;
        let mesh = torus(major, minor, 32, 16);
        assert!(mesh.is_valid());
        assert_eq!(mesh.vertices.len(), (32 * 16) as usize);
        assert_eq!(mesh.indices.len(), (32 * 16 * 6) as usize);

        // 表面点到管中心的方向即法线：pos - 管中心 = minor * normal。
        for vertex in &mesh.vertices {
            let length = dot(vertex.normal, vertex.normal).sqrt();
            assert!((length - 1.0).abs() < 1e-4, "圆环法线非单位长度: {length}");
            let radial_length = (vertex.position[0] * vertex.position[0]
                + vertex.position[1] * vertex.position[1])
                .sqrt();
            let radial = [
                vertex.position[0] / radial_length,
                vertex.position[1] / radial_length,
                0.0,
            ];
            let tube_center = [major * radial[0], major * radial[1], 0.0];
            let delta = sub(vertex.position, tube_center);
            assert!(
                (dot(delta, vertex.normal) - minor).abs() < 1e-3,
                "法线偏离管外方向: {:?} vs {:?}",
                delta,
                vertex.normal
            );
        }

        // 环面表面法线 = 归一化(pos - 管中心)；管中心沿 XY 径向位于主圆上。
        assert_winding_outward(&mesh, |v0, _| {
            let radial = (v0[0] * v0[0] + v0[1] * v0[1]).sqrt();
            let delta = sub(v0, [major * v0[0] / radial, major * v0[1] / radial, 0.0]);
            let length = dot(delta, delta).sqrt();
            [delta[0] / length, delta[1] / length, delta[2] / length]
        });
    }
}
