// v0.2 Cube：顶点缓冲 + 相机 UBO（binding 0）+ 逐实例 push constants（model/tint）。
// 单 .spv 多入口（-fvk-use-entrypoint-name）：入口名 vertexMain/fragmentMain
// 经 spirv 入口名保留，与 mir-render 管线的 `.name(b"vertexMain\0")` 等一致（ADR-0011）。

struct VertexInput
{
    [[vk::location(0)]] float3 position : POSITION;
    [[vk::location(1)]] float3 color : COLOR0;
};

struct VertexOutput
{
    float4 position : SV_Position;
    float3 color : TEXCOORD0;
};

struct FragmentInput
{
    float3 color : TEXCOORD0;
};

struct CameraUbo
{
    float4x4 view_proj;
};

[[vk::binding(0, 0)]]
ConstantBuffer<CameraUbo> camera;

struct PushConstants
{
    float4x4 model;
    float4 tint;
};

[[vk::push_constant]]
PushConstants pc;

[shader("vertex")]
VertexOutput vertexMain(VertexInput input)
{
    VertexOutput output;
    float4 world = mul(pc.model, float4(input.position, 1.0));
    output.position = mul(camera.view_proj, world);
    output.color = input.color;
    return output;
}

[shader("fragment")]
float4 fragmentMain(FragmentInput input) : SV_Target
{
    return float4(input.color * pc.tint.rgb, pc.tint.a);
}
