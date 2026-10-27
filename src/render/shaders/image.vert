// A textured quad in pixel space: the unit quad scaled to u_rect, uv across u_uv.
layout(location = 0) in vec2 a_pos;

uniform mat4 u_proj;
uniform vec4 u_rect;
uniform vec4 u_uv;

out vec2 v_uv;

void main() {
    vec2 p = u_rect.xy + a_pos * u_rect.zw;
    gl_Position = u_proj * vec4(p, 0.0, 1.0);
    v_uv = mix(u_uv.xy, u_uv.zw, a_pos);
}
