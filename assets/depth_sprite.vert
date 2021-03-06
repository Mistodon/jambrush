#version 450
#extension GL_ARB_separate_shader_objects : enable

layout(location = 0) in vec4 tint;
layout(location = 1) in vec2 uv;
layout(location = 2) in vec3 offset;
layout(location = 3) in vec2 depth_scale_add;

layout(location = 0) out vec4 vcolor;
layout(location = 1) out vec2 vuv;
layout(location = 2) out vec2 vdepth_scale_add;

void main() {
    vcolor = tint;
    vuv = uv;
    vdepth_scale_add = depth_scale_add;
    gl_Position = vec4(offset, 1.0);
}
