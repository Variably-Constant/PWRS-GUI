# Changes from wgpu-core 30.0.1

This is the wgpu-core crate, version 30.0.1, as crates.io publishes it,
under the licenses in `LICENSE.APACHE` and `LICENSE.MIT`, with four
changes:

- `Device::maintain` in `src/device/resource.rs` keeps whether its wait for
  a submission timed out, and reports the queue empty only when it did
  not: the change wgpu's pull request 9958 made on its trunk
  (https://github.com/gfx-rs/wgpu/pull/9958). In 30.0.1 a wait that timed
  out could read the fence before another thread polling the same device
  retired the submission, find the queue empty, and fail its assertion
  that the fence had reached the submission waited for ("If the queue is
  empty, the current submission index (106) should be at least the wait
  submission index (107)").
- `Device::create_texture_inner` in `src/device/resource.rs` refuses a
  multi-planar texture, NV12 or P010, with a dimension other than 2D or
  more than one mip level, through two errors `src/resource.rs` adds to
  `CreateTextureError`, `InvalidMultiPlanarDimension` and
  `InvalidMultiPlanarMipLevelCount`. Direct3D 12 allows neither shape for
  these formats, and Vulkan's valid usage asks formats that need a sampler
  Y'CbCr conversion for a 2D image of one mip level. 30.0.1 lets both
  through, and creating either loses the device on Direct3D 12.
- `clear_texture_via_buffer_copies` in `src/command/clear.rs` clears a
  multi-planar texture plane by plane, each plane at its own subsampled
  size: the change wgpu's pull request 9700 made on its trunk
  (https://github.com/gfx-rs/wgpu/pull/9700). In 30.0.1 it returned
  without clearing NV12 and P010 textures, and the render-pass clear finds
  one clear view for each mip level and layer where a multi-planar texture
  has one for each plane, so it cleared no chroma. `create_texture_inner`
  in `src/device/resource.rs` therefore clears every multi-planar texture
  by buffer copies, and `map_texture_usage_for_texture` in `src/conv.rs`
  gives it the copy destination usage they need. In 30.0.1 a new NV12
  texture read back what its memory held before.
- `Cargo.toml` allows the `unfulfilled_lint_expectations` lint. Cargo
  shows the warnings of a crate it builds from a path, and Rust 1.99 does
  not report the field `src/lock/ranked.rs` expects to be unused, which
  would otherwise print a warning in every build.
