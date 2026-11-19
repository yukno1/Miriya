// v0.1 Hello GPU：顶点/片元入口共用同一 Slang module，避免顶点缓冲与显存分配。
static const float2 POSITIONS[3] = {
    float2(0.0, -0.5),
    float2(0.5,  0.5),
    float2(-0.5,  0.5),
};

static const float3 COLORS[3] = {
    float3(1.0, 0.0, 0.0),
    float3(0.0, 1.0, 0.0),
    float3(0.0, 0.0, 1.0),
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

[shader("vertex")]
VertexOutput vertexMain(uint vertexId : SV_VertexID)
{
    VertexOutput output;
    output.position = float4(POSITIONS[vertexId], 0.0, 1.0);
    output.color = COLORS[vertexId];
    return output;
}

[shader("fragment")]
float4 fragmentMain(FragmentInput input) : SV_Target
{
    return float4(input.color, 1.0);
}
