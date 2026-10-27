// Premultiplied texels (so filtering doesn't darken edges), converted back to straight alpha
// for the blend and faded by u_alpha. highp: a tiled uv counts tiles (thousands for a
// tiny image), beyond mediump's precision on GLES.
in highp vec2 v_uv;

uniform sampler2D u_tex;
uniform float u_alpha;

out vec4 o_color;

void main() {
    vec4 c = texture(u_tex, v_uv);
    vec3 rgb = c.a > 0.0 ? c.rgb / c.a : vec3(0.0);
    o_color = vec4(rgb, c.a * u_alpha);
}
