//! The 3D view's colour grade (`grade::Grade`): colour fringes towards the edges,
//! exposure and tint, the sky's share of it, saturation, black crush, an S-curve, split
//! toning of shadows and highlights, a lift of the shadows, a vignette and film grain.
//! The time of day is in the numbers; this only applies them.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Grade {
    // rgb: tint times exposure.
    tint: vec4<f32>,
    // rgb: multiplies the bright upper part of the picture (the sky); a: how much.
    sky: vec4<f32>,
    // rgb: added to the shadows (linear); a: saturation.
    lift: vec4<f32>,
    // x: S-curve, y: vignette, z: grain, w: seconds (moves the grain).
    look: vec4<f32>,
    // rgb: shadows' split toning (perceptual offsets); a: black crush.
    shadows: vec4<f32>,
    // rgb: highlights' split toning; a: colour fringes at the corners (uv).
    highlights: vec4<f32>,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> grade: Grade;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.2126, 0.7152, 0.0722));
}

fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // Colour fringes: red and blue pulled apart, nothing in the middle, most at the corners.
    let centred = in.uv - 0.5;
    let shift = centred * (dot(centred, centred) * 2.0 * grade.highlights.a);
    let source = vec3(
        textureSample(screen, screen_sampler, in.uv + shift).r,
        textureSample(screen, screen_sampler, in.uv).g,
        textureSample(screen, screen_sampler, in.uv - shift).b,
    );
    var c = source * grade.tint.rgb;

    // The sky: bright and high up on the screen (the land is mostly darker and lower).
    let high = pow(clamp(1.0 - in.uv.y, 0.0, 1.0), 1.5);
    let bright = max(source.r, max(source.g, source.b));
    let sky = smoothstep(0.3, 0.6, bright) * high * grade.sky.a;
    c = mix(c, c * grade.sky.rgb, sky);

    c = max(mix(vec3(luma(c)), c, grade.lift.a), vec3(0.0));

    // In a roughly perceptual space: crushed blacks, the S-curve, then teal into the
    // shadows and warmth into the highlights.
    let crush = grade.shadows.a;
    var p = clamp((sqrt(c) - crush) / (1.0 - crush), vec3(0.0), vec3(1.0));
    p = mix(p, p * p * (3.0 - 2.0 * p), grade.look.x);
    let l = luma(p);
    p = p + grade.shadows.rgb * (1.0 - smoothstep(0.0, 0.55, l))
        + grade.highlights.rgb * smoothstep(0.4, 1.0, l);
    p = clamp(p, vec3(0.0), vec3(1.0));
    c = p * p;

    // Lifted shadows (a night's blue), fading out towards white.
    c = c + grade.lift.rgb * (1.0 - c);

    // Vignette, round whatever the window's shape.
    let size = vec2<f32>(textureDimensions(screen));
    let d = length(centred * vec2(size.x / size.y, 1.0));
    c = c * (1.0 - grade.look.y * smoothstep(0.35, 1.1, d));

    // Grain, stronger in the dark.
    let n = hash(in.uv * size + fract(grade.look.w) * 517.0) - 0.5;
    c = c + n * grade.look.z * (1.0 - clamp(luma(c) * 2.0, 0.0, 1.0));

    return vec4(max(c, vec3(0.0)), 1.0);
}
