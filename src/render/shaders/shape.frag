// Coverage from the distance to the shape's centre line: params = (kind, radius, half width).
in vec2 v_local;
in vec4 v_geom;
in vec4 v_params;
in vec4 v_color;

out vec4 o_color;

void main() {
    float d;
    if (v_params.x < 0.5) {
        // Line a..b with round caps.
        vec2 pa = v_local - v_geom.xy;
        vec2 ba = v_geom.zw - v_geom.xy;
        float h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-4), 0.0, 1.0);
        d = length(pa - ba * h);
    } else if (v_params.x < 1.5) {
        // Rounded rect outline: geom = (x, y, w, h).
        vec2 half_size = v_geom.zw * 0.5;
        vec2 q = abs(v_local - v_geom.xy - half_size) - half_size + v_params.y;
        d = abs(length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - v_params.y);
    } else {
        // Ring around geom.xy.
        d = abs(length(v_local - v_geom.xy) - v_params.y);
    }
    // Fragments sample pixel centres; a pixel half inside the stroke gets half coverage.
    float coverage = clamp(v_params.z - d + 0.5, 0.0, 1.0);
    o_color = vec4(v_color.rgb, v_color.a * coverage);
}
