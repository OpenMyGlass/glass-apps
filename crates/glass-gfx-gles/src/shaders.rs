//! GLSL ES 1.00 shaders. Every shader starts with `#version 100`; fragment shaders default
//! to `mediump`. Texture coordinates are `highp` varyings (when the fragment stage has
//! `highp`) passed straight to `texture2D`, with no arithmetic on them.

/// Attribute 0: position in pixels. Attribute 1: texture coordinate, or the shape's local
/// coordinate. `u_scale`/`u_bias` map pixels to clip space.
pub const VERTEX: &str = "#version 100
uniform vec2 u_scale;
uniform vec2 u_bias;
attribute vec2 a_pos;
attribute vec2 a_aux;
varying vec2 v_aux;
void main() {
    v_aux = a_aux;
    gl_Position = vec4(a_pos * u_scale + u_bias, 0.0, 1.0);
}
";

/// Solid shapes: a rounded rectangle drawn as four quadrants, each with `v_aux` = the
/// offset from its corner circle's centre, pointing outward (so it stays small, around
/// the corner radius, where it matters). The distance function runs in `highp` when the
/// fragment stage has it (the SGX540 does), else in `mediump`.
pub const SHAPE_FRAGMENT: &str = "#version 100
precision mediump float;
#ifdef GL_FRAGMENT_PRECISION_HIGH
#define LOCALP highp
#else
#define LOCALP mediump
#endif
varying LOCALP vec2 v_aux;
uniform LOCALP float u_radius;
uniform vec4 u_color;
void main() {
    LOCALP vec2 q = v_aux;
    LOCALP float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - u_radius;
    gl_FragColor = u_color * clamp(0.5 - d, 0.0, 1.0);
}
";

/// Textured quads (images, layers): premultiplied texels times the opacity.
pub const TEXTURE_FRAGMENT: &str = "#version 100
precision mediump float;
#ifdef GL_FRAGMENT_PRECISION_HIGH
varying highp vec2 v_aux;
#else
varying mediump vec2 v_aux;
#endif
uniform sampler2D u_tex;
uniform float u_opacity;
void main() {
    gl_FragColor = texture2D(u_tex, v_aux) * u_opacity;
}
";

/// Glyphs from the ALPHA atlas: the premultiplied colour times the coverage.
pub const GLYPH_FRAGMENT: &str = "#version 100
precision mediump float;
#ifdef GL_FRAGMENT_PRECISION_HIGH
varying highp vec2 v_aux;
#else
varying mediump vec2 v_aux;
#endif
uniform sampler2D u_tex;
uniform vec4 u_color;
void main() {
    gl_FragColor = u_color * texture2D(u_tex, v_aux).a;
}
";

#[cfg(test)]
mod tests {
    use crate::sgx::check_shader_source;

    #[test]
    fn every_shader_starts_with_version_100() {
        for s in [
            super::VERTEX,
            super::SHAPE_FRAGMENT,
            super::TEXTURE_FRAGMENT,
            super::GLYPH_FRAGMENT,
        ] {
            assert_eq!(check_shader_source(s), Ok(()));
        }
    }
}
