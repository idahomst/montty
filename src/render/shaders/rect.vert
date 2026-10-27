// Solid rects in pixel space: unit quad scaled per instance.
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec4 a_rect;
layout(location = 2) in vec4 a_color;

uniform mat4 u_proj;

out vec4 v_color;

void main() {
    vec2 p = a_rect.xy + a_pos * a_rect.zw;
    gl_Position = u_proj * vec4(p, 0.0, 1.0);
    v_color = a_color;
}
