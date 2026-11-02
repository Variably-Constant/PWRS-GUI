---
title: Rendering
weight: 60
---

The syntax, the parameters and the errors of `Invoke-WgpuRender` and `New-WgpuRenderPipeline`. The examples are on [Rendering](Rendering.md).

### Invoke-WgpuRender

Draws offscreen with a vertex and a fragment shader into a texture, or into an image file.

The shader is WGSL text or a Wgpu.Shader holding a vertex and a fragment entry point, or a pipeline from New-WgpuRenderPipeline; GLSL holds one stage, so it cannot draw. Each `@location` input of the vertex shader reads an array or buffer of its own from -Vertex, by name or in location order; a shader with no inputs draws -VertexCount vertices from vertex_index alone. -Binding gives the resources both stages use.

Alias: `Invoke-GpuRender`

```powershell
Invoke-WgpuRender [-Shader] <Object> [[-Width] <uint>] [[-Height] <uint>] [-Target <Texture>] [-Format <string>] [-Vertex <Object>] [-VertexCount <uint>] [-InstanceCount <uint>] [-Index <Object>] [-Binding <Object>] [-ClearColor <double[]>] [-Topology <PrimitiveTopology>] [-CullMode <CullMode>] [-Blend <BlendMode>] [-BlendSource <BlendFactor>] [-BlendDestination <BlendFactor>] [-BlendOperation <BlendOperation>] [-AlphaBlendSource <BlendFactor>] [-AlphaBlendDestination <BlendFactor>] [-AlphaBlendOperation <BlendOperation>] [-BlendConstant <double[]>] [-Depth] [-DepthFormat <string>] [-DepthCompare <CompareFunction>] [-DepthWrite] [-DepthBias <int>] [-DepthBiasSlopeScale <double>] [-DepthBiasClamp <double>] [-StencilCompare <CompareFunction>] [-StencilFail <StencilOperation>] [-StencilDepthFail <StencilOperation>] [-StencilPass <StencilOperation>] [-StencilReadMask <uint>] [-StencilWriteMask <uint>] [-BackStencilCompare <CompareFunction>] [-BackStencilFail <StencilOperation>] [-BackStencilDepthFail <StencilOperation>] [-BackStencilPass <StencilOperation>] [-StencilReference <uint>] [-ClearDepth <double>] [-ClearStencil <uint>] [-SampleCount <uint>] [-VertexEntry <string>] [-FragmentEntry <string>] [-Constant <IDictionary>] [-OutFile <string>] [-PassThru] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-AlphaBlendDestination` | [Wgpu.BlendFactor](Types.md#wgpublendfactor) |  | With -Blend Custom, what the alpha under the fragment is multiplied by; -BlendDestination when omitted. |
| `-AlphaBlendOperation` | [Wgpu.BlendOperation](Types.md#wgpublendoperation) |  | With -Blend Custom, how the two alpha products combine; -BlendOperation when omitted. |
| `-AlphaBlendSource` | [Wgpu.BlendFactor](Types.md#wgpublendfactor) |  | With -Blend Custom, what the fragment's alpha is multiplied by; -BlendSource when omitted. |
| `-BackStencilCompare` | [Wgpu.CompareFunction](Types.md#wgpucomparefunction) |  | -StencilCompare for triangles facing away; -StencilCompare's when omitted. |
| `-BackStencilDepthFail` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | -StencilDepthFail for triangles facing away; -StencilDepthFail's when omitted. |
| `-BackStencilFail` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | -StencilFail for triangles facing away; -StencilFail's when omitted. |
| `-BackStencilPass` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | -StencilPass for triangles facing away; -StencilPass's when omitted. |
| `-Binding` | object |  | The resources for the shaders' bindings. |
| `-Blend` | [Wgpu.BlendMode](Types.md#wgpublendmode) |  | How fragments combine with the target; Replace when omitted. Constant mixes by -BlendConstant, and Custom takes -BlendSource, -BlendDestination and -BlendOperation. |
| `-BlendConstant` | double[] |  | The color a blend factor of Constant reads, as red, green, blue and alpha; zero when omitted. |
| `-BlendDestination` | [Wgpu.BlendFactor](Types.md#wgpublendfactor) |  | With -Blend Custom, what the color under the fragment is multiplied by; Zero when omitted. |
| `-BlendOperation` | [Wgpu.BlendOperation](Types.md#wgpublendoperation) |  | With -Blend Custom, how the two products combine; Add when omitted. |
| `-BlendSource` | [Wgpu.BlendFactor](Types.md#wgpublendfactor) |  | With -Blend Custom, what the fragment's color is multiplied by; One when omitted. |
| `-ClearColor` | double[] |  | The color the target is cleared to, as red, green, blue and alpha; opaque black when omitted. |
| `-ClearDepth` | double |  | The depth the depth target starts at, from 0 to 1; 1 when omitted. |
| `-ClearStencil` | uint |  | The value the stencil starts at; 0 when omitted. |
| `-Constant` | IDictionary |  | Override constant values, by name or `@id`. |
| `-CullMode` | [Wgpu.CullMode](Types.md#wgpucullmode) |  | Which faces to discard; none when omitted. |
| `-Depth` | switch |  | Test and write depth in a Depth32Float target; a pipeline from New-WgpuRenderPipeline draws as it was built. |
| `-DepthBias` | int |  | A constant added to each triangle's depth, in the depth format's smallest steps; 0 when omitted. |
| `-DepthBiasClamp` | double |  | The largest bias the two give together, or the most negative when below 0; no limit when omitted. |
| `-DepthBiasSlopeScale` | double |  | A bias this many times the triangle's depth slope; 0 when omitted. |
| `-DepthCompare` | [Wgpu.CompareFunction](Types.md#wgpucomparefunction) |  | How a fragment's depth compares with the depth under it, for the fragment to be drawn; Less when omitted. |
| `-DepthFormat` | string |  | Test depth, stencil or both in a target of this format: Stencil8, Depth16Unorm, Depth24Plus, Depth24PlusStencil8, Depth32Float or Depth32FloatStencil8. |
| `-DepthWrite` | switch |  | Whether a fragment that passes the depth test writes its depth; it does unless -DepthWrite:$false. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to draw on; the default device when omitted. |
| `-Format` | string |  | The format of a new target texture; Rgba8Unorm when omitted. |
| `-FragmentEntry` | string |  | The fragment entry point; the shader's only one when omitted. |
| `-Height` | uint |  | The height of the texture drawn into, when -Target is not given. |
| `-Index` | object |  | Indices to draw by: uint[] or ushort[], or a Wgpu.Buffer of either. |
| `-InstanceCount` | uint |  | Instances to draw; 1 when omitted. |
| `-OutFile` | string |  | Save the result as an image file. |
| `-PassThru` | switch |  | Write the texture even when -OutFile is given. |
| `-SampleCount` | uint |  | Samples per pixel, resolved into the target; 1 when omitted. A pipeline from New-WgpuRenderPipeline draws with its own. |
| `-Shader` | object |  | WGSL text or a Wgpu.Shader with a vertex and a fragment entry point, or a Wgpu.RenderPipeline. |
| `-StencilCompare` | [Wgpu.CompareFunction](Types.md#wgpucomparefunction) |  | How the stencil test compares -StencilReference with the stencil value under a fragment, the reference on the left; Always when omitted. |
| `-StencilDepthFail` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | What a passed stencil test whose depth test fails does to the stencil value; Keep when omitted. |
| `-StencilFail` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | What a failed stencil test does to the stencil value; Keep when omitted. |
| `-StencilPass` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | What passing the stencil test and the depth test does to the stencil value; Keep when omitted. |
| `-StencilReadMask` | uint |  | The bits of the reference and of the stencil value the stencil test compares; all of them when omitted. |
| `-StencilReference` | uint |  | The value the stencil test compares with and Replace writes; 0 when omitted. A pipeline with no stencil test refuses it. |
| `-StencilWriteMask` | uint |  | The bits of the stencil value the stencil operations change; all of them when omitted. |
| `-Target` | [Wgpu.Texture](Types.md#wgputexture) |  | A texture to draw into instead of a new one. |
| `-Topology` | [Wgpu.PrimitiveTopology](Types.md#wgpuprimitivetopology) |  | How vertices make primitives; TriangleList when omitted. |
| `-Vertex` | object |  | The vertex inputs: a hashtable by input name, or arrays or buffers in location order. |
| `-VertexCount` | uint |  | Vertices to draw; from the first vertex input when omitted. |
| `-VertexEntry` | string |  | The vertex entry point; the shader's only one when omitted. |
| `-Width` | uint |  | The width of the texture drawn into, when -Target is not given. |

Writes [Wgpu.Texture](Types.md#wgputexture).

#### Errors

The target: `WgpuNoShader`, `WgpuNoTarget`, `WgpuUnknownFormat` and `WgpuTexture`. The shader: `WgpuShaderInvalid`, `WgpuNoEntryPoint`, `WgpuEntryPointAmbiguous`, `WgpuVertexInput`, `WgpuLayout`, `WgpuShader` and `WgpuRenderPipeline`. The pipeline's tests and blend: `WgpuDepthFormat`, `WgpuTextureFormat`, `WgpuDepthTest`, `WgpuStencil`, `WgpuDepthBias` and `WgpuBlend`. The inputs: `WgpuNoVertices`, `WgpuVertexCount`, `WgpuNoData`, `WgpuNotNumeric`, `WgpuOutOfRange`, `WgpuBuffer`, `WgpuClearColor`, `WgpuClearDepth`, `WgpuClearStencil`, `WgpuBlendConstant`, `WgpuStencilReference`, and from `-Binding` the errors [Invoke-WgpuCompute](Compute-Cmdlets.md#invoke-wgpucompute) lists for resources. The draw: `WgpuDraw`, `WgpuSubmit`, `WgpuSubmitThread`, `WgpuPoll` and `WgpuStopped`. `-OutFile`: `WgpuImage`, `WgpuImageExtension`, `WgpuImageFormat`, `WgpuImageShape`, `WgpuImageWrite`, `WgpuTextureRead`, `WgpuRead` and `WgpuMap`. Without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. Each is described in the [Error Reference](Error-Reference.md).

### New-WgpuRenderPipeline

Builds a render pipeline from a shader with a vertex and a fragment entry point, for one device and one target format.

Alias: `New-GpuRenderPipeline`

```powershell
New-WgpuRenderPipeline [-Shader] <Object> [-Format <string[]>] [-Depth] [-DepthFormat <string>] [-DepthCompare <CompareFunction>] [-DepthWrite] [-DepthBias <int>] [-DepthBiasSlopeScale <double>] [-DepthBiasClamp <double>] [-StencilCompare <CompareFunction>] [-StencilFail <StencilOperation>] [-StencilDepthFail <StencilOperation>] [-StencilPass <StencilOperation>] [-StencilReadMask <uint>] [-StencilWriteMask <uint>] [-BackStencilCompare <CompareFunction>] [-BackStencilFail <StencilOperation>] [-BackStencilDepthFail <StencilOperation>] [-BackStencilPass <StencilOperation>] [-Topology <PrimitiveTopology>] [-CullMode <CullMode>] [-Blend <BlendMode>] [-BlendSource <BlendFactor>] [-BlendDestination <BlendFactor>] [-BlendOperation <BlendOperation>] [-AlphaBlendSource <BlendFactor>] [-AlphaBlendDestination <BlendFactor>] [-AlphaBlendOperation <BlendOperation>] [-SampleCount <uint>] [-VertexEntry <string>] [-FragmentEntry <string>] [-Constant <IDictionary>] [-Label <string>] [-Device <Device>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-AlphaBlendDestination` | [Wgpu.BlendFactor](Types.md#wgpublendfactor) |  | With -Blend Custom, what the alpha under the fragment is multiplied by; -BlendDestination when omitted. |
| `-AlphaBlendOperation` | [Wgpu.BlendOperation](Types.md#wgpublendoperation) |  | With -Blend Custom, how the two alpha products combine; -BlendOperation when omitted. |
| `-AlphaBlendSource` | [Wgpu.BlendFactor](Types.md#wgpublendfactor) |  | With -Blend Custom, what the fragment's alpha is multiplied by; -BlendSource when omitted. |
| `-BackStencilCompare` | [Wgpu.CompareFunction](Types.md#wgpucomparefunction) |  | -StencilCompare for triangles facing away; -StencilCompare's when omitted. |
| `-BackStencilDepthFail` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | -StencilDepthFail for triangles facing away; -StencilDepthFail's when omitted. |
| `-BackStencilFail` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | -StencilFail for triangles facing away; -StencilFail's when omitted. |
| `-BackStencilPass` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | -StencilPass for triangles facing away; -StencilPass's when omitted. |
| `-Blend` | [Wgpu.BlendMode](Types.md#wgpublendmode) |  | How fragments combine with the target; Replace when omitted. Constant mixes by the constant a draw sets, and Custom takes -BlendSource, -BlendDestination and -BlendOperation. |
| `-BlendDestination` | [Wgpu.BlendFactor](Types.md#wgpublendfactor) |  | With -Blend Custom, what the color under the fragment is multiplied by; Zero when omitted. |
| `-BlendOperation` | [Wgpu.BlendOperation](Types.md#wgpublendoperation) |  | With -Blend Custom, how the two products combine; Add when omitted. |
| `-BlendSource` | [Wgpu.BlendFactor](Types.md#wgpublendfactor) |  | With -Blend Custom, what the fragment's color is multiplied by; One when omitted. |
| `-Constant` | IDictionary |  | Override constant values, by name or `@id`. |
| `-CullMode` | [Wgpu.CullMode](Types.md#wgpucullmode) |  | Which faces to discard; none when omitted. |
| `-Depth` | switch |  | Test and write depth in a Depth32Float target. |
| `-DepthBias` | int |  | A constant added to each triangle's depth, in the depth format's smallest steps; 0 when omitted. |
| `-DepthBiasClamp` | double |  | The largest bias the two give together, or the most negative when below 0; no limit when omitted. |
| `-DepthBiasSlopeScale` | double |  | A bias this many times the triangle's depth slope; 0 when omitted. |
| `-DepthCompare` | [Wgpu.CompareFunction](Types.md#wgpucomparefunction) |  | How a fragment's depth compares with the depth under it, for the fragment to be drawn; Less when omitted. |
| `-DepthFormat` | string |  | Test depth, stencil or both in a target of this format: Stencil8, Depth16Unorm, Depth24Plus, Depth24PlusStencil8, Depth32Float or Depth32FloatStencil8. |
| `-DepthWrite` | switch |  | Whether a fragment that passes the depth test writes its depth; it does unless -DepthWrite:$false. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | The device to build it on; the default device when omitted. |
| `-Format` | string[] |  | The formats of the color targets; Rgba8Unorm when omitted. |
| `-FragmentEntry` | string |  | The fragment entry point; the shader's only one when omitted. |
| `-Label` | string |  | The pipeline's label. |
| `-SampleCount` | uint |  | Samples per pixel; 1 when omitted. |
| `-Shader` | object | by value | WGSL text or a Wgpu.Shader with a vertex and a fragment entry point. |
| `-StencilCompare` | [Wgpu.CompareFunction](Types.md#wgpucomparefunction) |  | How the stencil test compares the stencil reference, which a pass sets, with the stencil value under a fragment, the reference on the left; Always when omitted. |
| `-StencilDepthFail` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | What a passed stencil test whose depth test fails does to the stencil value; Keep when omitted. |
| `-StencilFail` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | What a failed stencil test does to the stencil value; Keep when omitted. |
| `-StencilPass` | [Wgpu.StencilOperation](Types.md#wgpustenciloperation) |  | What passing the stencil test and the depth test does to the stencil value; Keep when omitted. |
| `-StencilReadMask` | uint |  | The bits of the reference and of the stencil value the stencil test compares; all of them when omitted. |
| `-StencilWriteMask` | uint |  | The bits of the stencil value the stencil operations change; all of them when omitted. |
| `-Topology` | [Wgpu.PrimitiveTopology](Types.md#wgpuprimitivetopology) |  | How vertices make primitives; TriangleList when omitted. |
| `-VertexEntry` | string |  | The vertex entry point; the shader's only one when omitted. |

Writes [Wgpu.RenderPipeline](Types.md#wgpurenderpipeline).

#### Errors

`WgpuNoShader`, `WgpuUnknownFormat`, `WgpuDepthFormat`, `WgpuTextureFormat`, `WgpuDepthTest`, `WgpuStencil`, `WgpuDepthBias`, `WgpuBlend`, `WgpuShaderInvalid`, `WgpuNoEntryPoint`, `WgpuEntryPointAmbiguous`, `WgpuVertexInput`, `WgpuLayout`, `WgpuShader` and `WgpuRenderPipeline`, and without `-Device`, opening the default device: `WgpuLoader`, `WgpuNoAdapter` and `WgpuDevice`. Each is described in the [Error Reference](Error-Reference.md), as is PWRS's `PwrsConversionError` for a `-Constant` value that is not a number.
