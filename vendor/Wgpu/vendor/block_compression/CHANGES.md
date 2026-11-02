# Changes from block_compression 0.11.0

These are the WGSL shaders of the block_compression crate, version
0.11.0, under the license in `LICENSE`, with four changes:

- `compute_axis` in `bc6h.wgsl` and `bc7.wgsl`, and `compute_axis3` in
  `bc1_to_5.wgsl`, start their power iteration from the covariance column
  of the channel that varies most, where block_compression starts from a
  vector of ones. A vector of ones is orthogonal to the principal axis of
  a block whose channels change by amounts that sum to zero, such as a
  gradient from red to blue, and the axis the iteration finds for such a
  block then depends on how the GPU's compiler rounds; on Apple's GPUs it
  stays on the vector of ones and the block encodes as one flat color.
- `bc7.wgsl` and `bc6h.wgsl` call each of their encoders from one place.
  A shader holds a copy of a function for every place that calls it, and
  Direct3D 12's software adapter, WARP, crashes, stops responding or
  writes wrong blocks when it compiles block_compression's, whose BC7
  shader calls its partition encoder from five places and whose BC6H
  shader calls its mode test, which encodes, from twenty. In `bc7.wgsl`,
  modes 0, 2, 1, 3 and 7 share one partition-ranking loop, one search of
  their partitions and one refinement, in place of `bc7_enc_mode02`,
  `bc7_enc_mode13` and `bc7_enc_mode7`, and `bc7_enc_mode45` tries modes 4
  and 5 through one call of `bc7_enc_mode45_candidate`. In `bc6h.wgsl`,
  `compress_bc6h_core` runs
  its trials as the steps of one loop with one call each of
  `bc6h_test_mode`, which tests a mode without encoding it, `bc6h_enc_2p`,
  `bc6h_enc_1p` and `bc6h_code`, which writes the block in place of
  `bc6h_code_2p` and `bc6h_code_1p`; the encoders return what they found
  instead of writing it. The trials run in block_compression's order with
  its arithmetic. BC6H's blocks are the same on an NVIDIA RTX 5070 and an
  AMD Radeon, through Vulkan and Direct3D 12; BC7's are the same on the
  RTX 5070, and on the Radeon a few differ and decode no more than half a
  percent farther from their source, because a mode the shader reads at
  run time, instead of a constant in each copy, changes how the GPU's
  compiler rounds.
- `bc7.wgsl` encodes in four passes, in place of `compress_bc7`, which
  the caller dispatches in this order over the same blocks, with the state
  they share for each block in buffers at bindings 4 to 6:
  `compress_bc7_mode456` tries modes 4, 5 and 6; `compress_bc7_rank`
  ranks the partitions for modes 1, 3 and 7; `compress_bc7_search` finds
  each partitioned mode's best partition; and `compress_bc7_finish`
  refines and packs the partitioned modes and writes the best block,
  choosing in block_compression's order of modes 0, 2, 1, 3, 7, 4, 5 and
  6, where a mode replaces the block of one tried before it only with a
  smaller error. WARP on Arm64 cannot compile a shader the size of the
  whole encoder, even with each function called from one place, and
  compiles each pass. The blocks are the same as those of the encoder in
  one pass on an NVIDIA RTX 5070 and an AMD Radeon, through Vulkan and
  Direct3D 12, and on WARP on x64.
- `ep_quant0367` in `bc7.wgsl` keeps mode 6's p-bit at 1 in an opaque
  encode, whose settings have three channels. Mode 6 stores each endpoint as seven
  bits a channel and one p-bit its four channels share, and
  block_compression chooses the p-bit by the error of red, green and blue
  alone, so where p-bit 0 suits the colors the alpha endpoints, 255,
  quantize to 254, and up to three quarters of a smooth gradient's
  texels decode with alpha 254. With p-bit 1 every texel decodes with
  alpha 255, and the colors' endpoints take odd values only, at most one
  step from the even ones.
