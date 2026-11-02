---
title: Tutorials
weight: 1
sidebar:
  open: true
---

From installing the module to drawing offscreen, one short chapter at a time, with the cmdlets and, where they work differently, the objects' methods side by side. Read the pages in order:

1. [Getting Started](Getting-Started.md) - install the module, see the GPUs it finds, run a first shader and read what comes back.
2. [Arrays And Buffers](Arrays-And-Buffers.md) - send arrays to the GPU, keep numbers there in buffers, and work on them without writing a shader.
3. [Compute Shaders](Compute-Shaders.md) - write a compute shader in WGSL, give it arrays and settings, decide how many times it runs, and build it once to run again.
4. [Shaders Without A GPU](Shaders-Without-A-GPU.md) - check a shader and read its mistakes, read what it declares, and translate it into the languages other APIs take.
5. [Textures And Images](Textures-And-Images.md) - make a texture, save it as an image file and load it back, run a shader over every texel, and resize it.
6. [Drawing Offscreen](Drawing-Offscreen.md) - draw a triangle into a PNG, feed the shaders corners from arrays, let depth decide what is in front, and blend.
7. [Commands And Tasks](Commands-And-Tasks.md) - record steps into one submission, run many dispatches in one pass, and go on in PowerShell while the GPU works.
