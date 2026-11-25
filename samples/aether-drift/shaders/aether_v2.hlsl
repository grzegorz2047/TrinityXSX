// SPDX-License-Identifier: MIT
// AETHER DRIFT v2.1 — Real-Time GPU Raymarched Volumetric Storm Clouds,
// Analytical Height-Fog + Swirling Megacity Mist Layers, Wet-Track Mirror Reflections,
// Blender PBR Craft & HDR Quarter-Res Bloom.

cbuffer FrameCB : register(b0) {
    float4x4 viewProj;
    float4 camPosTime;   // xyz = camPos, w = time
    float4 fogParams;    // rgb = fog color, w = fog density
    float4 sunDirGlow;   // xyz = sun dir, w = boost glow [0..1]
};

Texture2D texPrimary : register(t0);
Texture2D texPuddle  : register(t1);
Texture2D texSky     : register(t2);

SamplerState sampWrap  : register(s0);
SamplerState sampClamp : register(s1);

struct VSIn {
    float3 pos    : POSITION;
    float3 normal : NORMAL;
    float2 uv     : TEXCOORD0;
    float4 col    : COLOR;
    float2 mat    : TEXCOORD1;
};

struct PSIn {
    float4 pos      : SV_Position;
    float3 worldPos : TEXCOORD0;
    float3 normal   : NORMAL;
    float2 uv       : TEXCOORD1;
    float4 col      : COLOR;
    float2 mat      : TEXCOORD2;
};

// ---------------------------------------------------------------------------
// Fast GPU 3D Value Noise + Billowing FBM for Real-Time Volumetric Clouds & Fog
// ---------------------------------------------------------------------------
float Hash31(float3 p) {
    p = frac(p * float3(0.1031, 0.1030, 0.0973));
    p += dot(p, p.yxz + 33.33);
    return frac((p.x + p.y) * p.z);
}

float Noise3D(float3 p) {
    float3 i = floor(p);
    float3 f = frac(p);
    f = f * f * (3.0 - 2.0 * f);

    float n000 = Hash31(i + float3(0, 0, 0));
    float n100 = Hash31(i + float3(1, 0, 0));
    float n010 = Hash31(i + float3(0, 1, 0));
    float n110 = Hash31(i + float3(1, 1, 0));
    float n001 = Hash31(i + float3(0, 0, 1));
    float n101 = Hash31(i + float3(1, 0, 1));
    float n011 = Hash31(i + float3(0, 1, 1));
    float n111 = Hash31(i + float3(1, 1, 1));

    float nx00 = lerp(n000, n100, f.x);
    float nx10 = lerp(n010, n110, f.x);
    float nx01 = lerp(n001, n101, f.x);
    float nx11 = lerp(n011, n111, f.x);

    float nxy0 = lerp(nx00, nx10, f.y);
    float nxy1 = lerp(nx01, nx11, f.y);
    return lerp(nxy0, nxy1, f.z);
}

float CloudFBM(float3 p) {
    float v = 0.0;
    float a = 0.50;
    [unroll]
    for (int k = 0; k < 5; ++k) {
        float n = Noise3D(p);
        // Blend smooth puffs with billowing cumulus ridge domes
        float billow = 1.0 - abs(n * 2.0 - 1.0);
        v += a * lerp(n, billow * billow, 0.42);
        p = p * 2.12 + float3(13.1, 7.7, 5.3);
        a *= 0.50;
    }
    return v;
}

// Fast 2D scrolling mist density for volumetric city fog layers
float MistFBM(float2 p) {
    return CloudFBM(float3(p.x, 0.35, p.y));
}

float2 DirToEquirectUV(float3 dir) {
    float3 d = normalize(dir);
    float u = atan2(d.z, d.x) / (2.0 * 3.14159265) + 0.5;
    float v = asin(clamp(d.y, -1.0, 1.0)) / 3.14159265;
    return float2(u, 0.5 - v);
}

// ---------------------------------------------------------------------------
// Analytical Volumetric Height-Fog + Swirling City Mist + Twilight In-Scattering
// ---------------------------------------------------------------------------
float3 ApplyVolumetricFog(float3 color, float3 worldPos, float emissiveKeep) {
    float3 rel = worldPos - camPosTime.xyz;
    float dist = length(rel);
    float3 rayDir = rel / max(dist, 0.001);

    // 1. Exponential atmospheric distance aerial perspective
    float distFog = 1.0 - exp(-dist * (fogParams.w * 1.35));

    // 2. Dense volumetric cyan-blue city mist rising right up to elevated track height (y < 64)!
    float mistTop = 64.0;
    float subDepth = saturate((mistTop - worldPos.y) / 135.0);
    float wisp = MistFBM(worldPos.xz * 0.011 + float2(camPosTime.w * 0.04, -camPosTime.w * 0.025));
    float heightFog = saturate(subDepth * (0.68 + 0.72 * wisp) * (1.0 - exp(-dist * 0.0055)));

    // 3. Twilight sun forward in-scattering (warm violet/amber atmospheric haze toward sun!)
    float3 L = normalize(sunDirGlow.xyz);
    float sunAmt = pow(saturate(dot(rayDir, L)), 4.0);

    // Blend luminous cyan-indigo city mist below with moody twilight violet-amber haze above
    float3 lowMistCol = float3(0.14, 0.30, 0.44) + float3(0.10, 0.22, 0.30) * wisp;
    float3 highHazeCol = fogParams.rgb * 1.25 + float3(0.58, 0.26, 0.42) * sunAmt * 0.85;
    float3 fogCol = lerp(highHazeCol, lowMistCol, saturate(subDepth * 1.5));

    float totalFog = saturate(distFog * 0.88 + heightFog * 0.94);
    totalFog *= lerp(1.0, 0.28, saturate(emissiveKeep));
    return lerp(color, fogCol, totalFog);
}

// ---------------------------------------------------------------------------
// 1. Standard 3D World Vertex Shader
// ---------------------------------------------------------------------------
PSIn WorldVS(VSIn v) {
    PSIn o;
    o.pos = mul(viewProj, float4(v.pos, 1.0));
    o.worldPos = v.pos;
    o.normal = v.normal;
    o.uv = v.uv;
    o.col = v.col;
    o.mat = v.mat;
    return o;
}

// ---------------------------------------------------------------------------
// 2. REAL-TIME GPU VOLUMETRIC RAYMARCHED 3D STORM CLOUDS SKY DOME!
// ---------------------------------------------------------------------------
PSIn SkyVS(VSIn v) {
    PSIn o;
    float3 wp = v.pos * 1800.0 + camPosTime.xyz;
    o.pos = mul(viewProj, float4(wp, 1.0));
    o.pos.z = o.pos.w * 0.99995;
    o.worldPos = v.pos;
    o.normal = v.normal;
    o.uv = v.uv;
    o.col = v.col;
    o.mat = v.mat;
    return o;
}

float SampleCloudDensity(float3 p, float hNorm, float t) {
    // Dramatic towering storm-cloud anvil shaping: crisp thunderhead bases + puffy tops
    float heightProfile = smoothstep(0.0, 0.18, hNorm) * smoothstep(1.0, 0.32, hNorm);
    float3 q = p * float3(0.0022, 0.0032, 0.0022) + float3(t * 0.022, 0.0, t * 0.014);
    float base = CloudFBM(q);
    // High-frequency detail erosion for sharp volumetric cloud edges!
    float detail = Noise3D(q * 4.4 - float3(t * 0.04, t * 0.015, 0.0));
    float d = base * heightProfile - 0.22 - detail * 0.18;
    return saturate(d * 3.4);
}

float4 SkyPS(PSIn i) : SV_Target {
    float3 dir = normalize(i.worldPos);
    float t = camPosTime.w;

    float3 sunDir = normalize(float3(0.58, 0.10, 0.80));
    float cosTheta = dot(dir, sunDir);

    // Rich twilight storm sky gradient
    float yPos = max(dir.y, 0.0);
    float3 zenith = float3(0.03, 0.025, 0.11);
    float3 midSky = float3(0.13, 0.075, 0.25);
    float3 horizonCol = float3(0.44, 0.22, 0.38);
    float3 sky = lerp(horizonCol, midSky, pow(saturate(yPos * 2.2), 0.72));
    sky = lerp(sky, zenith, pow(saturate(yPos * 1.15), 1.35));

    float horizonGlow = exp(-abs(dir.y - 0.02) * 10.5);
    float sunAz = pow(saturate(cosTheta * 0.5 + 0.5), 2.2);
    sky += float3(0.92, 0.42, 0.26) * horizonGlow * (0.28 + 0.90 * sunAz);
    sky += float3(1.0, 0.68, 0.42) * pow(saturate(cosTheta), 24.0) * 0.85 * smoothstep(-0.05, 0.12, dir.y);

    // REAL 3D VOLUMETRIC CLOUD RAYMARCHING (22 steps, high-contrast storm thunderheads!)
    if (dir.y > 0.01) {
        float cloudBot = 165.0;
        float cloudTop = 520.0;
        float dy = max(dir.y, 0.048);
        float tMin = cloudBot / dy;
        float tMax = min(cloudTop / dy, 3600.0);
        const int STEPS = 22;
        float dt = (tMax - tMin) / (float)STEPS;

        float g = 0.62;
        float hg = (1.0 - g * g) / (4.0 * 3.14159 * pow(max(0.05, 1.0 + g * g - 2.0 * g * cosTheta), 1.5));
        float phase = 0.30 + saturate(hg * 1.55);

        // Deep stormy dark indigo-purple cloud bellies vs bright silver-violet & sunset peach lit rims!
        float3 litSunCol = lerp(float3(0.68, 0.48, 0.98), float3(1.30, 0.62, 0.38), sunAz * horizonGlow * 1.45 + pow(saturate(cosTheta), 3.5) * 0.65);
        float3 darkCloudCol = float3(0.035, 0.025, 0.11);
        float3 ambientTop = float3(0.32, 0.26, 0.56);

        float3 accumLight = float3(0.0, 0.0, 0.0);
        float transmittance = 1.0;

        float jitter = Hash31(dir * 512.0);
        float marchT = tMin + dt * jitter;

        [loop]
        for (int s = 0; s < STEPS; ++s) {
            if (transmittance < 0.025) break;
            float3 samplePos = dir * marchT;
            float hNorm = saturate((samplePos.y - cloudBot) / (cloudTop - cloudBot));
            float dens = SampleCloudDensity(samplePos, hNorm, t);

            if (dens > 0.005) {
                // Dual-tap directional self-shadow march toward twilight sun for crisp thunderhead contrast!
                float3 sp1 = samplePos + sunDir * 48.0;
                float3 sp2 = samplePos + sunDir * 125.0;
                float dSun = SampleCloudDensity(sp1, saturate((sp1.y - cloudBot) / (cloudTop - cloudBot)), t) * 0.65
                           + SampleCloudDensity(sp2, saturate((sp2.y - cloudBot) / (cloudTop - cloudBot)), t) * 0.45;
                float shadow = exp(-dSun * 4.2);
                float powder = 1.0 - exp(-dens * 2.8);

                float3 stepLight = lerp(darkCloudCol, ambientTop, hNorm * 0.78)
                                 + litSunCol * shadow * phase * (0.55 + 1.15 * powder);

                float alpha = 1.0 - exp(-dens * dt * 0.0068);
                accumLight += transmittance * stepLight * alpha;
                transmittance *= (1.0 - alpha);
            }
            marchT += dt;
        }

        float horizonFade = smoothstep(0.01, 0.12, dir.y);
        sky = lerp(sky, accumLight + sky * transmittance, horizonFade);
    }

    // Below horizon: dense swirling luminous cyan-indigo megacity fog ocean!
    if (dir.y < 0.05) {
        float below = smoothstep(0.05, -0.14, dir.y);
        float2 mistUV = dir.xz / max(0.12, -dir.y + 0.22) * 1.45 + float2(t * 0.024, -t * 0.018);
        float sw = MistFBM(mistUV);
        float3 abyssCol = lerp(float3(0.05, 0.12, 0.22), float3(0.18, 0.36, 0.50), sw);
        sky = lerp(sky, abyssCol, below);
    }

    return float4(sky, 1.0);
}

// ---------------------------------------------------------------------------
// 3. Wet Metallic Racing Track (albedo + puddle mask + real sky reflections!)
// ---------------------------------------------------------------------------
float4 TrackPS(PSIn i) : SV_Target {
    float3 N = normalize(i.normal);
    float3 V = normalize(camPosTime.xyz - i.worldPos);

    float4 alb = texPrimary.Sample(sampWrap, i.uv);
    float4 pud = texPuddle.Sample(sampWrap, i.uv * float2(1.0, 0.5));
    float puddle = pud.r;

    float2 pert = (pud.ba - 0.5) * 0.35 * (1.0 - puddle * 0.85);
    float3 Nw = normalize(N + float3(pert.x, 0.0, pert.y));

    float3 L = normalize(sunDirGlow.xyz);
    float NdotL = saturate(dot(Nw, L));
    float3 H = normalize(L + V);
    float specPow = lerp(28.0, 220.0, puddle);
    float spec = pow(saturate(dot(Nw, H)), specPow) * lerp(0.25, 1.85, puddle);

    float3 R = reflect(-V, Nw);
    R.y = abs(R.y) * 0.85 + 0.04;
    float3 skyRefl = texSky.Sample(sampWrap, DirToEquirectUV(R)).rgb;

    float NdotV = saturate(dot(Nw, V));
    float fresnel = pow(1.0 - NdotV, 3.0);
    float reflStrength = lerp(0.18, 0.88, puddle) * (0.32 + 0.68 * fresnel);

    float edgeDist = abs(frac(i.uv.x) - 0.5) * 2.0;
    float edgeGlow = pow(saturate(edgeDist), 3.0) * (0.45 + 0.55 * puddle);
    float side = step(0.5, frac(i.uv.x));
    float3 railBounce = lerp(float3(0.14, 0.88, 1.0), float3(0.98, 0.25, 0.82), side) * edgeGlow * 0.52;

    float3 baseCol = alb.rgb * i.col.rgb * lerp(0.72, 0.42, puddle);
    float3 lit = baseCol * (float3(0.24, 0.30, 0.44) + NdotL * float3(1.05, 0.90, 0.82) * 0.9)
               + skyRefl * reflStrength * 1.25
               + float3(0.95, 0.92, 1.0) * spec
               + railBounce * (1.0 + sunDirGlow.w * 0.35);

    lit += i.col.rgb * i.mat.y * 1.45;
    lit = ApplyVolumetricFog(lit, i.worldPos, i.mat.y * 0.4);
    return float4(lit, 1.0);
}

// ---------------------------------------------------------------------------
// 4. Sculpted Blender Hovercraft PBR (metallic clearcoat + sky env reflection + HDR glow!)
// ---------------------------------------------------------------------------
float4 CraftPS(PSIn i) : SV_Target {
    float3 N = normalize(i.normal);
    float3 V = normalize(camPosTime.xyz - i.worldPos);
    float3 L = normalize(sunDirGlow.xyz);

    float metal = i.mat.x;
    float emissive = i.mat.y;

    float NdotL = saturate(dot(N, L));
    float hemi = N.y * 0.5 + 0.5;
    float3 amb = lerp(float3(0.08, 0.14, 0.26), float3(0.28, 0.36, 0.54), hemi);

    float3 R = reflect(-V, N);
    float3 envRefl = texSky.Sample(sampWrap, DirToEquirectUV(R)).rgb;
    float NdotV = saturate(dot(N, V));
    float fresnel = pow(1.0 - NdotV, 3.0);

    float3 H = normalize(L + V);
    float spec = pow(saturate(dot(N, H)), lerp(24.0, 140.0, metal)) * lerp(0.25, 1.65, metal);

    float underBounce = saturate(-N.y * 0.5 + 0.5) * 0.26;
    float3 neonRim = float3(0.25, 0.88, 1.0) * (fresnel * 0.38 + underBounce);

    float3 surface = i.col.rgb * (amb + NdotL * float3(1.0, 0.94, 0.88) * 0.85)
                   + envRefl * (0.18 + 0.55 * fresnel) * metal * 1.10
                   + float3(0.95, 0.98, 1.0) * spec * 0.80
                   + neonRim * 0.65;

    float3 color = lerp(surface, i.col.rgb * (1.35 + emissive * 0.95), saturate(emissive * 0.75));
    color = ApplyVolumetricFog(color, i.worldPos, emissive * 0.5);
    return float4(color, i.col.a);
}

// ---------------------------------------------------------------------------
// 5. True Wet-Track Planar Mirror Reflection Pass (Stencil == 1 on track surface!)
// ---------------------------------------------------------------------------
float4 ReflPS(PSIn i) : SV_Target {
    float puddle = texPuddle.Sample(sampWrap, i.worldPos.xz * 0.018).r;
    float mask = lerp(0.18, 0.78, puddle);

    float dist = length(i.worldPos - camPosTime.xyz);
    float distFade = saturate(1.0 - dist / 420.0);

    float3 col = i.col.rgb * (0.38 + i.mat.y * 0.62) * mask * distFade * i.col.a;
    col *= float3(0.84, 0.95, 1.05);
    return float4(col * 0.44, 1.0);
}

// ---------------------------------------------------------------------------
// 6. Cyberpunk Skyscraper Facade (Blender Spires emerging from Volumetric City Mist!)
// ---------------------------------------------------------------------------
float4 CityPS(PSIn i) : SV_Target {
    float3 N = normalize(i.normal);
    float3 L = normalize(sunDirGlow.xyz);

    float4 tex = texPrimary.Sample(sampWrap, i.uv);
    float isWindow = smoothstep(0.55, 0.85, tex.a);

    float NdotL = saturate(dot(N, L));
    float3 wall = tex.rgb * i.col.rgb * (float3(0.13, 0.17, 0.28) + NdotL * 0.42);
    float3 winGlow = tex.rgb * (1.65 + i.mat.y * 0.7);
    float3 color = lerp(wall, winGlow, isWindow);
    color = lerp(color, i.col.rgb * (1.45 + i.mat.y * 0.75), saturate(i.mat.y * 0.6));

    color = ApplyVolumetricFog(color, i.worldPos, max(isWindow * 0.35, i.mat.y * 0.35));
    return float4(color, 1.0);
}

// ---------------------------------------------------------------------------
// 7. Trackside Glowing Billboards & Animated Turn Chevrons
// ---------------------------------------------------------------------------
float4 BillboardPS(PSIn i) : SV_Target {
    float2 uv = i.uv;
    if (i.mat.x > 0.5) {
        uv.x = frac(uv.x - camPosTime.w * 0.65);
    }
    float4 tex = texPrimary.Sample(sampClamp, uv);
    float3 color = tex.rgb * i.col.rgb * (1.35 + i.mat.y * 0.45);
    color = ApplyVolumetricFog(color, i.worldPos, 0.72);
    return float4(color, tex.a * i.col.a);
}

// ---------------------------------------------------------------------------
// 8. Volumetric Swirling Megacity Mist Banks (drifting glowing cloud sheets weaving around skyscrapers!)
// ---------------------------------------------------------------------------
float4 MistPS(PSIn i) : SV_Target {
    float t = camPosTime.w;
    float2 p = i.worldPos.xz * 0.0085;
    float n1 = MistFBM(p + float2(t * 0.032, -t * 0.018));
    float n2 = MistFBM(p * 2.3 - float2(t * 0.022, t * 0.028));
    float cloud = smoothstep(0.22, 0.76, n1 * 0.62 + n2 * 0.38);

    float dist = length(i.worldPos - camPosTime.xyz);
    float nearFade = smoothstep(22.0, 82.0, dist);
    float farFade = 1.0 - smoothstep(520.0, 1550.0, dist);

    // Luminous cyberpunk city-lit volumetric fog puffs: cyan-blue backlit with soft violet glow
    float3 mistCol = lerp(float3(0.14, 0.30, 0.46), float3(0.46, 0.68, 0.84), n2) * i.col.rgb;
    float alpha = cloud * i.col.a * nearFade * farFade;
    return float4(mistCol, saturate(alpha));
}

// ---------------------------------------------------------------------------
// 9. Volumetric Additive FX (Thruster Plumes, Ribbon Trails, Rain Streaks, Sparks)
// ---------------------------------------------------------------------------
float4 FxPS(PSIn i) : SV_Target {
    float3 color = i.col.rgb * (1.20 + i.mat.y * 0.75);
    color = ApplyVolumetricFog(color, i.worldPos, 0.80);
    return float4(color, i.col.a);
}

// ---------------------------------------------------------------------------
// 10. HDR Post-Processing: Quarter-Res Bloom Extract, Separable Blur, ACES Composite
// ---------------------------------------------------------------------------
cbuffer PostCB : register(b0) {
    float4 postParams;
};

struct PostVSOut {
    float4 pos : SV_Position;
    float2 uv  : TEXCOORD0;
};

PostVSOut FullscreenVS(uint vid : SV_VertexID) {
    PostVSOut o;
    float2 p = float2((vid == 1) ? 3.0 : -1.0, (vid == 2) ? -3.0 : 1.0);
    o.pos = float4(p, 0.0, 1.0);
    o.uv = float2(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return o;
}

float4 BloomExtractPS(PostVSOut i) : SV_Target {
    float2 d = postParams.xy * 1.5;
    float3 c = (
        texPrimary.Sample(sampClamp, i.uv + float2(-d.x, -d.y)).rgb +
        texPrimary.Sample(sampClamp, i.uv + float2( d.x, -d.y)).rgb +
        texPrimary.Sample(sampClamp, i.uv + float2(-d.x,  d.y)).rgb +
        texPrimary.Sample(sampClamp, i.uv + float2( d.x,  d.y)).rgb
    ) * 0.25;
    float luma = dot(c, float3(0.2126, 0.7152, 0.0722));
    float soft = saturate((luma - 0.82) / 0.55);
    return float4(c * soft * soft, 1.0);
}

float4 BloomBlurHPS(PostVSOut i) : SV_Target {
    float2 step = float2(postParams.x * 1.65, 0.0);
    float w[5] = { 0.227027, 0.194595, 0.121622, 0.054054, 0.016216 };
    float3 acc = texPrimary.Sample(sampClamp, i.uv).rgb * w[0];
    [unroll]
    for (int k = 1; k < 5; ++k) {
        acc += texPrimary.Sample(sampClamp, i.uv + step * k).rgb * w[k];
        acc += texPrimary.Sample(sampClamp, i.uv - step * k).rgb * w[k];
    }
    return float4(acc, 1.0);
}

float4 BloomBlurVPS(PostVSOut i) : SV_Target {
    float2 step = float2(0.0, postParams.y * 1.65);
    float w[5] = { 0.227027, 0.194595, 0.121622, 0.054054, 0.016216 };
    float3 acc = texPrimary.Sample(sampClamp, i.uv).rgb * w[0];
    [unroll]
    for (int k = 1; k < 5; ++k) {
        acc += texPrimary.Sample(sampClamp, i.uv + step * k).rgb * w[k];
        acc += texPrimary.Sample(sampClamp, i.uv - step * k).rgb * w[k];
    }
    return float4(acc, 1.0);
}

float3 AcesFilm(float3 x) {
    return saturate((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14));
}

float4 CompositePS(PostVSOut i) : SV_Target {
    float2 centered = i.uv - 0.5;
    float r2 = dot(centered, centered);

    float ca = (0.0007 + postParams.w * 0.0042) * r2;
    float3 scene;
    scene.r = texPrimary.Sample(sampClamp, i.uv + centered * ca).r;
    scene.g = texPrimary.Sample(sampClamp, i.uv).g;
    scene.b = texPrimary.Sample(sampClamp, i.uv - centered * ca).b;

    float3 bloom = texPuddle.Sample(sampClamp, i.uv).rgb;
    float3 hdr = scene + bloom * postParams.z;

    float vig = 1.0 - r2 * (0.40 + postParams.w * 0.22);
    hdr *= saturate(vig);

    float3 ldr = AcesFilm(hdr * 1.08);
    return float4(ldr, 1.0);
}
