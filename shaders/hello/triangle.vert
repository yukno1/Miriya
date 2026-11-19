#version 450

// v0.1 Hello GPU：用 gl_VertexIndex 生成三角形，避免顶点缓冲与显存分配。
const vec2 POSITIONS[3] = vec2[3](
    vec2( 0.0, -0.5),
    vec2( 0.5,  0.5),
    vec2(-0.5,  0.5)
);

const vec3 COLORS[3] = vec3[3](
    vec3(1.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0),
    vec3(0.0, 0.0, 1.0)
);

layout(location = 0) out vec3 out_color;

void main() {
    gl_Position = vec4(POSITIONS[gl_VertexIndex], 0.0, 1.0);
    out_color = COLORS[gl_VertexIndex];
}
