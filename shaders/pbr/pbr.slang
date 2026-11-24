// PBR（R-12）：解析式 metallic-roughness（GGX 近似）+ 单方向光 + 固定半球环境项。
// v0.3 边界（m2-remaining.md 决策 6）：光源方向/颜色为着色器内常量；IBL/色调映射归 M5。
// 单 .spv 多入口（-fvk-use-entrypoint-name）：入口名 vertexMain/fragmentMain 与
// mir-render 管线的 `.name(b"vertexMain\0")` 等一致（ADR-0011）。
//
// 布局契约（与 crates/mir-render/src 一一对应，修改须同步）：
// - Vertex：position@0 / color@1 / normal@2，stride 36（mesh.rs）
// - CameraUbo：set 0 binding 0，std140：view_proj（0…63）+ camera_position（64…75），80 B
// - MaterialUbo：set 1 binding 0，std140：base_color（0…15）/ metallic（16）/
//   roughness（20）/ emissive（32…43），48 B（material.rs MaterialData）
// - PushConstants：model（64 B）+ tint（16 B），80 B

struct VertexInput
{
    [[vk::location(0)]] float3 position : POSITION;
    [[vk::location(1)]] float3 color : COLOR0;
    [[vk::location(2)]] float3 normal : NORMAL;
};

struct VertexOutput
{
    float4 position : SV_Position;
    float3 world_pos : TEXCOORD0;
    float3 normal : TEXCOORD1;
    float3 color : TEXCOORD2;
};

struct CameraUbo
{
    float4x4 view_proj;
    float3 camera_position;
};

[[vk::binding(0, 0)]]
ConstantBuffer<CameraUbo> camera;

struct MaterialUbo
{
    float4 base_color;
    float metallic;
    float roughness;
    float3 emissive;
};

[[vk::binding(0, 1)]]
ConstantBuffer<MaterialUbo> material;

struct PushConstants
{
    float4x4 model;
    float4 tint;
};

[[vk::push_constant]]
PushConstants pc;

// 单方向光（世界系，Z 轴向上；方向指向光源）。光强随批次 F 验收可调。
static const float3 LIGHT_DIRECTION = normalize(float3(0.30, -0.40, 0.86));
static const float3 LIGHT_COLOR = float3(3.2, 3.0, 2.8);

// 固定半球环境项（天空偏冷、地面偏暖；Z 轴向上插值）。
static const float3 AMBIENT_SKY = float3(0.14, 0.16, 0.20);
static const float3 AMBIENT_GROUND = float3(0.09, 0.07, 0.06);

static const float PI = 3.14159265358979323846;

// GGX/Trowbridge-Reitz 法线分布。
float d_ggx(float noh, float alpha)
{
    float a2 = alpha * alpha;
    float d = noh * noh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Schlick-GGX 可见性（UE 移动端近似：k = alpha / 2）。
float visibility_smith(float nov, float nol, float alpha)
{
    float k = alpha * 0.5;
    float g_v = nov / (nov * (1.0 - k) + k);
    float g_l = nol / (nol * (1.0 - k) + k);
    return g_v * g_l;
}

// Schlick 菲涅尔；f0 = 电介质 0.04 与金属 base_color 按金属度插值。
float3 fresnel_schlick(float vog, float3 f0)
{
    return f0 + (1.0 - f0) * pow(1.0 - vog, 5.0);
}

[shader("vertex")]
VertexOutput vertexMain(VertexInput input)
{
    VertexOutput output;
    float4 world = mul(pc.model, float4(input.position, 1.0));
    output.position = mul(camera.view_proj, world);
    output.world_pos = world.xyz;
    // TRS 均匀缩放下 mat3(model) 变换后归一化即可恢复单位法线。
    output.normal = normalize(mul(pc.model, float4(input.normal, 0.0)).xyz);
    output.color = input.color;
    return output;
}

[shader("fragment")]
float4 fragmentMain(VertexOutput input) : SV_Target
{
    float3 n = normalize(input.normal);
    float3 v = normalize(camera.camera_position - input.world_pos);
    float3 l = LIGHT_DIRECTION;
    float3 h = normalize(v + l);

    float nov = max(dot(n, v), 1e-4);
    float nol = max(dot(n, l), 0.0);
    float noh = max(dot(n, h), 0.0);
    float vog = max(dot(v, h), 0.0);

    // 粗糙度重映射（UE 惯例）避免 alpha → 0 的数值问题。
    float alpha = material.roughness * material.roughness;

    float3 base_color = material.base_color.rgb * input.color * pc.tint.rgb;
    float3 f0 = lerp(float3(0.04, 0.04, 0.04), base_color, material.metallic);

    // 直接光：Lambert 漫反射 + GGX 镜面反射。
    float3 diffuse = base_color / PI * (1.0 - material.metallic);
    float3 specular = d_ggx(noh, alpha)
        * visibility_smith(nov, nol, alpha)
        * fresnel_schlick(vog, f0);
    float3 direct = (diffuse + specular) * LIGHT_COLOR * nol;

    // 环境项：固定半球插值，金属按 f0 近似（无 IBL）。
    float up = n.z * 0.5 + 0.5;
    float3 ambient = lerp(AMBIENT_GROUND, AMBIENT_SKY, up)
        * lerp(base_color, f0, material.metallic);

    float3 color = direct + ambient + material.emissive;
    return float4(color, material.base_color.a * pc.tint.a);
}
