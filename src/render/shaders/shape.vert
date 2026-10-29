// Antialiased strokes: a unit quad per instance covering the stroke; geometry is
// relative to the quad's corner.
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec4 a_quad;
layout(location = 2) in vec4 a_geom;
layout(location = 3) in vec4 a_params;
layout(location = 4) in vec4 a_color;

uniform mat4 u_proj;

out vec2 v_local;
out vec4 v_geom;
out vec4 v_params;
out vec4 v_color;

void main() {
    v_local = a_pos * a_quad.zw;
    gl_Position = u_proj * vec4(a_quad.xy + v_local, 0.0, 1.0);
    v_geom = a_geom;
    v_params = a_params;
    v_color = a_color;
}
