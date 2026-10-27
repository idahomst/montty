// Cell instances. Pass 0: backgrounds (wide cells span two cells).
// Pass 1: glyph quads at origin + (col*cw + left, row*ch + baseline - top).
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_cell;   // col, row
layout(location = 2) in vec4 a_bg;
layout(location = 3) in vec4 a_fg;
layout(location = 4) in vec4 a_uv;     // atlas x, y, w, h (texels)
layout(location = 5) in vec2 a_offset; // glyph left, top
layout(location = 6) in uint a_flags;  // 1 = COLOR_GLYPH, 2 = WIDE

uniform mat4 u_proj;
uniform vec2 u_origin;
uniform vec2 u_cell;
uniform float u_baseline;
uniform vec2 u_atlas;
uniform int u_pass;

out vec4 v_color;
out highp vec2 v_uv;
// 0 = background, 1 = tinted mask glyph, 2 = color glyph
flat out uint v_mode;

void main() {
    vec2 cell = u_origin + a_cell * u_cell;
    vec2 p;
    if (u_pass == 0) {
        float span = (a_flags & 2u) != 0u ? 2.0 : 1.0;
        p = cell + a_pos * vec2(u_cell.x * span, u_cell.y);
        v_color = a_bg;
        v_uv = vec2(0.0);
        v_mode = 0u;
    } else {
        vec2 top_left = cell + vec2(a_offset.x, u_baseline - a_offset.y);
        p = top_left + a_pos * a_uv.zw;
        v_color = a_fg;
        v_uv = (a_uv.xy + a_pos * a_uv.zw) / u_atlas;
        v_mode = (a_flags & 1u) != 0u ? 2u : 1u;
    }
    gl_Position = u_proj * vec4(p, 0.0, 1.0);
}
