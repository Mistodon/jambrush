Library
===

Currently only designing the top layer of the library. It will need split into more fine-grained layers once the API has settled a little.

## Swapchain management

- [ ] Configure how the canvas scales with the window size
    - [x] Allow pixel-perfect scaling
    - [ ] Allow disabling scaling entirely
    - [ ] Allow fill-window scaling
    - [ ] Allow free-aspect in combination with any of these
- [ ] Configure canvas scaling mode (linear / nearest)
- [ ] Should scaling be tied to screen coordinates?
    - What if I want a 256x144 pixel-perfect canvas, but my coordinate system to be [0.0, 0.0] to [16.0, 9.0]?
    - What if I want my coordinate system to be free-aspect with 0,0 in the middle?
- [ ] Allow drawing straight onto the surface without a separate canvas texture?
    - Is this necessary? Just a small(?) optimization.

## Sprites

- [ ] Allow preloading/loading multiple sprites at once.
- [ ] Find a nice way to layer the API
    - At its simplest, fn sprite(sprite, pos, layer)
    - At its most complex, fn sprite(sprite, pos, layer, tint, lighting, scale, rotation)
- [ ] Add a little shortcut for animation frames (Sheet[i] = Sprite)
- [ ] Review sprite shader - optimize and simplify
- [ ] Actually batch sprites in a vertex buffer
- [ ] Configure sprite scaling mode (linear / nearest)
- [ ] Configure pixel-density of a sprite

## Backends

- [x] Metal backend
- [ ] Native backend (Metal, DX12, Vulkan)
- [ ] OpenGL option
- [ ] Dynamic selection between native/OpenGL based on hardware support

## Primitives

- [ ] Filled rectangles
- [ ] Filled circles
- [ ] Filled polygons
- [ ] Line rectangles
- [ ] Line circles
- [ ] Line polygons

## Render context

- [ ] Camera offset
- [ ] Viewport/scissor rect

## Stencil

- [ ] Allow drawing primitives into stencil buffer and masking out drawing with it

## Off-canvas drawing

- [ ] Consider allowing rendering onto the main surface, outside canvas boundary
    - Maybe this is too high level - this could already be implemented with free
      aspect and viewport/camera for the inner "canvas"

## Build-time

- [ ] Allow generating the texture atlas offline and loading it all at once

Examples
===

## Simple API examples

- [ ] Pixel-perfect sprites
- [ ] Free-aspect with non pixel-perfect sprites and linear blending
- [ ] Kitchen sink example with sprites, primitives, stencil, etc.

## Layered API examples

- [ ] Custom instance/adapter setup
- [ ] Custom swapchain management
- [ ] Custom pipeline/shaders
- [ ] Custom rendering
- [ ] Mix of managed and custom