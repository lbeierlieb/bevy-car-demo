//! The 3D view's colour grade (`graphics::Grade`): exposure and tint, the sky's share
//! of it, saturation, contrast, a lift of the shadows, a vignette and film grain. The
//! time of day is in the numbers; this only applies them.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Grade {
    // rgb: tint times exposure.
    tint: vec4<f32>,
    // rgb: multiplies the bright upper part of the picture (the sky); a: how much.
    sky: vec4<f32>,
    // rgb: added to the shadows; a: saturation.
    lift: vec4<f32>,
    // x: contrast, y: vignette, z: grain, w: seconds (moves the grain).
    look: vec4<f32>,
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
    let source = textureSample(screen, screen_sampler, in.uv).rgb;
    var c = source * grade.tint.rgb;

    // The sky: bright and high up on the screen (the land is mostly darker and lower).
    let high = pow(clamp(1.0 - in.uv.y, 0.0, 1.0), 1.5);
    let bright = max(source.r, max(source.g, source.b));
    let sky = smoothstep(0.3, 0.6, bright) * high * grade.sky.a;
    c = mix(c, c * grade.sky.rgb, sky);

    // Saturation, then contrast around mid grey in a roughly perceptual space.
    c = max(mix(vec3(luma(c)), c, grade.lift.a), vec3(0.0));
    var p = sqrt(c);
    p = (p - 0.5) * grade.look.x + 0.5;
    c = max(p, vec3(0.0));
    c = c * c;

    // Lifted shadows (a night's blue), fading out towards white.
    c = c + grade.lift.rgb * (1.0 - clamp(c, vec3(0.0), vec3(1.0)));

    // Vignette, round whatever the window's shape.
    let size = vec2<f32>(textureDimensions(screen));
    let d = length((in.uv - 0.5) * vec2(size.x / size.y, 1.0));
    c = c * (1.0 - grade.look.y * smoothstep(0.35, 1.1, d));

    // Grain, stronger in the dark.
    let n = hash(in.uv * size + fract(grade.look.w) * 517.0) - 0.5;
    c = c + n * grade.look.z * (1.0 - clamp(luma(c) * 2.0, 0.0, 1.0));

    return vec4(max(c, vec3(0.0)), 1.0);
}
