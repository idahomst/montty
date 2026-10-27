// Straight (non-premultiplied) alpha throughout; blending is SRC_ALPHA, ONE_MINUS_SRC_ALPHA.
in vec4 v_color;
in highp vec2 v_uv;
flat in uint v_mode;

uniform sampler2D u_tex;

out vec4 o_color;

void main() {
    if (v_mode == 0u) {
        o_color = v_color;
    } else if (v_mode == 1u) {
        o_color = vec4(v_color.rgb, v_color.a * texture(u_tex, v_uv).a);
    } else {
        o_color = texture(u_tex, v_uv);
    }
}
