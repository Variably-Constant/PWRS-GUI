---
title: Error Reference
weight: 110
---

Every one of the 167 error ids the module and its Rust library raise, with its category and its messages. Source: the calls in `src/` that raise them, through `invalid`, `not_found`, `invalid_operation`, `device_error`, `unresolved` and `input_files` in `src/error.rs`, through `Core::scoped` in `src/device.rs`, which raises what wgpu reports while a call runs, through `Core::submit_as` and `Core::begin_submit` there, which raise what wgpu reports of a submission, or through `PsError::new`, read by `tools/Export-Reference.ps1`.

A cmdlet writes an error as a non-terminating error record and goes on with its next input. The record's `FullyQualifiedErrorId` is the id, a comma and the class of the cmdlet, as `WgpuNoAdapter,Pwrs.Modules.Wgpu.OpenWgpuDeviceCommand`, and its `CategoryInfo.Category` is the category below. A method throws instead: PowerShell wraps the error in a `MethodInvocationException`, whose `InnerException` is a `Pwrs.PwrsException` with a message that begins with the id in brackets, as `[WgpuRange] element 5 is past the end of a buffer of 3 elements`. The examples below read both.

Text in angle brackets is filled in when the error is raised, and `<reason>` is what wgpu, the driver, naga or the operating system reported, in its own words. A path PowerShell cannot resolve is reported with PowerShell's own reason, most often the one shown. A call that reaches the device checks it before and after, so most can also fail with `WgpuDeviceLost` once the device is lost, or with `WgpuUncaptured` when the device reported errors outside any call since it was last checked.

## The ids

| Id | Category | Message |
|---|---|---|
| `WgpuAdjust` | InvalidArgument | `<-Brightness or -Contrast> runs from -100 to 100, as ImageMagick's -brightness-contrast, not <value>`, or `-Gamma is a number above 0, not <value>`, or `-Exposure is a finite number of stops, not <value>`, or `Adjust takes a brightness, a contrast, a gamma or an exposure` |
| `WgpuAlpha` | InvalidArgument | `<format> has no alpha channel, so -KeepCoverage would change nothing`, or `<format> has no alpha channel, so <parameter> would change nothing`, or `<without> has no alpha channel, so -KeepCoverage would change nothing`, or `<from> has no alpha channel, so -FixAlphaBorder would change nothing` |
| `WgpuAlphaThreshold` | InvalidArgument | `-AlphaThreshold is above 0 and at most 1, not <value>` |
| `WgpuAspect` | InvalidArgument | `<format> keeps its texels in planes, which are read one at a time: Read-WgpuTexture reads its luma with -Aspect Plane0 and its chroma with -Aspect Plane1`, or `<format> has no planes: -Aspect Plane0 and Plane1 name the planes of NV12 and P010`, or `<format> keeps its texels in planes: give -Aspect Plane0 for its luma or Plane1 for its chroma`, or `<format> needs -Aspect DepthOnly or StencilOnly for its texels to be read or written`, or `only the stencil of <format> can be read or written, with -Aspect StencilOnly: its depth has no layout outside the GPU`, or `the depth of <format> has no layout outside the GPU, so it cannot be read or written` |
| `WgpuAttachment` | InvalidArgument | `a render attachment is a Wgpu.Texture or a Wgpu.TextureView, not <type>` |
| `WgpuBasisEncode` | InvalidArgument | `Basis Universal could not allocate its compression parameters`, or `Basis Universal did not take image <index>, <width>x<height>: <what Basis Universal printed>`, or `Basis Universal could not encode the texture in the format chosen: <what Basis Universal printed>`, or `Basis Universal encodes 2D textures, arrays and cube maps, not 1D or 3D ones` |
| `WgpuBindGroup` | DeviceError | `the resources do not fit the shader's bindings: <reason>` |
| `WgpuBindingCount` | InvalidArgument | `the shader uses <n> bindings (<bindings>) and <n> resources were given; give one per binding in that order, or a hashtable by name` |
| `WgpuBindingKey` | InvalidArgument | `<key> is neither a binding name nor group:binding: <reason>` |
| `WgpuBindingMissing` | InvalidArgument | `no resource was given for <bindings>`, or `no resource was given for <name> (<group>:<binding>): its value is $null`, or `no resource was given for <name> (<group>:<binding>)` |
| `WgpuBindingTwice` | InvalidArgument | `<key> names <name> (<group>:<binding>), which another resource already binds` |
| `WgpuBlend` | InvalidArgument | `-BlendSource, -BlendDestination, -BlendOperation and their alpha forms name a Custom blend, and the blend is <mode>: give -Blend Custom` |
| `WgpuBlendConstant` | InvalidArgument | `the blend reads a constant: give the draw a blend_constant`, or `a draw's blend_constant sets the constant a blend reads, and this blend reads none`, or `a blend constant is four numbers, red, green, blue and alpha; <n> were given`, or `the blend reads a constant: give -BlendConstant`, or `-BlendConstant sets the constant a blend reads, and this blend reads none` |
| `WgpuBlur` | InvalidArgument | `-Blur takes standard deviations above 0, and <sigma> is not`, or `-Blur takes one standard deviation, or two, across and down, and was given <n>` |
| `WgpuBorder` | DeviceError or InvalidArgument | `-BorderReach limits -FixAlphaBorder: give -FixAlphaBorder`, or `-BorderReach is at least 1 texel; omit it to fill every transparent texel`, or `the border is fixed in Rgba16Float or Rgba32Float, not <type>`, or `the border cannot be fixed: <reason>`, or `-FixAlphaBorder colors texels with alpha 0, and a premultiplied texel with alpha 0 stays black: give -FixAlphaBorder or -Premultiply, not both` |
| `WgpuBrdfTable` | InvalidArgument | `-Size is the table's side in texels, 1 or more` |
| `WgpuBuffer` | DeviceError | `the buffer cannot be created: <reason>`, or `the new buffer cannot be written: <reason>` |
| `WgpuBufferTooLarge` | InvalidArgument | `<size> bytes is more than the largest buffer <adapter> creates, <max buffer size> bytes`, or `<length> elements of <type> is more bytes than a buffer can address`, or `the read gives <count> elements, more than a .NET array holds: <reason>` |
| `WgpuClear` | DeviceError | `the buffer cannot be cleared: <reason>` |
| `WgpuClearColor` | InvalidArgument | `a clear color is three or four numbers, red, green, blue and alpha; <n> were given`, or `ClearColor is red, green, blue and alpha, or 'Load': <message>` |
| `WgpuClearDepth` | InvalidArgument | `<-ClearDepth or clear_depth> sets the depth the depth target starts at, and <what the depth target lacks>`, or `<-ClearDepth or clear_depth> is a depth from 0 to 1, and <value> is not`, or `ClearDepth sets what the depth target's depth starts at, and <what the depth target lacks>`, or `ClearDepth is a depth from 0 to 1 or 'Load': <message>`, or `ClearDepth is a depth from 0 to 1 or 'Load', and <number> is not` |
| `WgpuClearStencil` | InvalidArgument | `<-ClearStencil or clear_stencil> sets the value the stencil starts at, and <what the depth target lacks>`, or `ClearStencil sets what the depth target's stencil starts at, and <what the depth target lacks>`, or `ClearStencil is a stencil value or 'Load': <message>` |
| `WgpuColorKey` | InvalidArgument | `-ColorKeyTolerance widens -ColorKey: give -ColorKey`, or `-ColorKeyTolerance runs from 0 to 1, in the units of the key's channels, not <t>`, or `-ColorKey is red, green and blue from 0 to 1, or '#RRGGBB', as '#FF00FF'; '<text>' is not that`, or `-ColorKey '<text>' is not hexadecimal: <reason>`, or `-ColorKey is red, green and blue from 0 to 1, or '#RRGGBB'; <n> numbers were given, <numbers>`, or `ColorKey takes a color: red, green and blue from 0 to 1, or '#RRGGBB'` |
| `WgpuColorSpace` | InvalidArgument | `-AssignColorSpace relabels a format that has an sRGB form, as Rgba8Unorm has Rgba8UnormSrgb, and <format> has none` |
| `WgpuCompare` | InvalidArgument | `the reference is a <reference dimension> texture and the texture a <texture dimension> one: textures compare only with the same dimension`, or `the reference is <width>x<height> and the texture <width>x<height>: textures compare only at the same width and height`, or `the reference has <reference layers> and the texture <texture layers>: textures compare only with the same <depth or number of layers>`, or `the reference is <reference format> and the texture <texture format>: textures compare only when both keep their texels in planes or neither does`, or `the textures do not both store <channel>: they share <channels>` |
| `WgpuCompress` | DeviceError or InvalidArgument | `a level is too large to encode: <reason>`, or `the encoder cannot be built: <reason>`, or `the texture cannot be prepared for encoding: <reason>`, or `a <format> texture is read as floating-point colors to encode it, which this device cannot do`, or `encoding a texture reads it in a shader: it needs TextureBinding among its usages`, or `<format> has no block layout`, or `the texture cannot be encoded: <reason>`, or `a level is too large to encode in one buffer: <reason>` |
| `WgpuCompressFormat` | InvalidArgument | `the module encodes BC6H unsigned only: give Bc6hRgbUfloat, for colors of zero and above`, or `<format> is not a format the module encodes on the GPU; it encodes Bc1RgbaUnorm, Bc2RgbaUnorm, Bc3RgbaUnorm and Bc7RgbaUnorm and their Srgb forms, Bc4RUnorm, Bc4RSnorm, Bc5RgUnorm, Bc5RgSnorm and Bc6hRgbUfloat, and ASTC and ETC2 textures come from the Basis Universal formats, such as BasisAstc4x4 and BasisEtc1s`, or `Compress takes a BC format or a Basis Universal format, not <format>` |
| `WgpuConvert` | DeviceError or InvalidArgument | `the texture cannot be converted: <reason>`, or `mip levels and format conversions read texels as floating-point colors, and <format> holds integers, depth or stencil, which a box filter cannot average`, or `<format> texels render in no float format` |
| `WgpuCopy` | DeviceError | `the copy is invalid: <reason>` |
| `WgpuCopyAlignment` | InvalidArgument | `a buffer copy moves whole 4-byte words; the <offset or size>, <value> bytes, is not a multiple of 4` |
| `WgpuCoverage` | DeviceError | `the coverage cannot be counted: <reason>`, or `the coverage cannot be kept: <reason>` |
| `WgpuCrop` | InvalidArgument | `-Crop takes four numbers, the left, top, width and height of a region, or two, a width and height the anchor places; <n> were given`, or `-CropMargin takes four numbers, the texels cut from the left, top, right and bottom; <n> were given`, or `give -Crop or -CropMargin, not both`, or `the region at <x>,<y> of <width>x<height> texels is not inside the <texture width>x<texture height> texture`, or `a crop to <width>x<height> texels does not fit inside the <texture width>x<texture height> texture`, or `margins of <left>, <top>, <right> and <bottom> texels leave nothing of the <texture width>x<texture height> texture` |
| `WgpuCube` | InvalidArgument | `cube faces are square 2D layers, six to a cube; a <width>x<height> file with <layers> layers is not that`, or `cube faces are square 2D layers, six to a cube; a <width>x<height> texture with <shape> is not that` |
| `WgpuCubeMap` | DeviceError or InvalidArgument | `a <layout> is <cells across and down, or square>, which a <width>x<height> image is not`, or `a square image may be an angular map or an octahedral map: give -Layout AngularMap or -Layout Octahedral for this <width>x<height> one`, or `a <width>x<height> image has no shape a cube comes in: <the aspect ratio of each shape>; -Layout names a tee, a turned cross, an angular map or an octahedral map`, or `a <width>x<height> image holds no faces`, or `a <format> texture has no layout faces can be cut from`, or `a <format> texture has no layout faces can be laid out in`, or `the texture a cube map pass renders into cannot be made: <reason>`, or `a cube map pass cannot be made: <reason>`, or `a cube map pass cannot read its texture: <reason>`, or `a cube map pass cannot be drawn: <reason>`, or `a cube map pass cannot be submitted: <reason>`, or `<what is made> reads texels as floating-point colors, and <format> holds integers, depth or stencil`, or `-Yaw is a finite number of degrees, not <value>`, or `-Yaw and -FaceSize apply to a panorama, an angular map or an octahedral map, which are resampled; a <layout>'s faces are its cells as they are, and Resize-WgpuTexture changes their size`, or `-FaceSize is the faces' size in texels, 1 or more`, or `a cube is made from one 2D image with one layer, or from six textures, its faces; this texture has <its layers or dimension>`, or `a cube map is six square layers, its faces, and this texture holds <what it holds>`, or `-Layout and -Yaw read a cube from one image, and this texture is a cube's six faces already`, or `a cube is made from six faces, and none was given`, or `a cube is made from six faces, and <n> were given`, or `<face> is not a 2D texture with one layer, as a face is`, or `<face> is <width>x<height>, and a cube's faces are square`, or `<face> is <size and format> where <the first face> is <its size and format>: a cube's faces share one size and format, and Resize-WgpuTexture or Convert-WgpuTexture -Format brings a face in line`, or `-Layout, -Yaw and -FaceSize read a cube from one image; six textures are its faces as they are`, or `a cube is made from one image holding its faces or from six textures, its faces, and <n> were given`, or `<face> was not given: a cube takes all six faces`, or `-Layout and -Panorama each name the image; give one`, or `-Layout, -Yaw and -FaceSize say how -Cube reads a cube from the file: give -Cube`, or `-Layout, -Yaw and -FaceSize read a cube from one image, and the file holds <layers> layers`, or `-Yaw and -FaceSize say how -Layout or -Panorama writes a cube into an image file: give one of them`, or `-Layout and -Panorama write a cube into an image file; a .dds or .ktx2 file holds its faces as they are` |
| `WgpuDecoder` | InvalidArgument | `-Decoder applies to a block-compressed texture, and <format> texels need no decoding`, or `-Decoder applies to a texture saved as it is; -Layout and -Panorama decode a block-compressed cube's faces on the CPU`, or `-Decoder applies to an image file; a DDS or KTX2 file holds a block-compressed texture's blocks as they are` |
| `WgpuDepthBias` | InvalidArgument | `a depth bias moves triangles, and the topology is <topology>` |
| `WgpuDepthFormat` | InvalidArgument | `a depth_format is a depth or stencil format, and <format> is not one`, or `-DepthFormat names a depth or stencil format, one of <the depth and stencil formats>, and <format> is not one` |
| `WgpuDepthTest` | InvalidArgument | `-DepthCompare, -DepthWrite and the depth bias test depth, and <what the depth target lacks>: give -Depth or a -DepthFormat with depth` |
| `WgpuDestination` | InvalidArgument | `no array was given to read into`, or `a read of <type> elements fills a <array type>, and the destination is a <destination type>`, or `the read gives <count> elements, and the destination holds <length>` |
| `WgpuDevice` | DeviceError | `the device cannot be opened: <reason>`, or `the device's submission thread cannot start: <reason>` |
| `WgpuDeviceLost` | DeviceError | `the device was lost: <reason>` |
| `WgpuDispatch` | DeviceError | `the dispatch is invalid: <reason>`, or `the expression cannot run: <reason>`, or `the reduction cannot run: <reason>` |
| `WgpuDispatchSize` | InvalidArgument | `<parameter> takes one to three counts, x, y and z`, or `give one of -Workgroups, -Invocations and -Indirect` |
| `WgpuDpi` | InvalidArgument | `-Dpi is a number of dots per inch above 0, not <dpi>` |
| `WgpuDraw` | DeviceError | `the draw is invalid: <reason>` |
| `WgpuEdges` | InvalidArgument | `-EdgeThreshold and -EdgeSigma say how Canny finds edges: give -Edges Canny`, or `-EdgeThreshold is Canny's low threshold, 0 or above, and its high one, no lower, and <low> and <high> are not`, or `-EdgeThreshold takes two numbers, Canny's low and high thresholds, and was given <n>`, or `-EdgeSigma is the standard deviation of the smoothing before Canny's gradient, above 0, and <sigma> is not` |
| `WgpuEffort` | InvalidArgument | `-Effort applies to Bc7RgbaUnorm, Bc7RgbaUnormSrgb and Bc6hRgbUfloat, and <format> has a single setting`, or `-Effort runs from 0 to 10, not <effort>`, or `-Effort applies with a -Format to encode in: a Basis Universal format, Bc7RgbaUnorm, Bc7RgbaUnormSrgb or Bc6hRgbUfloat` |
| `WgpuEmpty` | InvalidArgument | `the array for <name> is empty`, or `a buffer needs at least one element`, or `there are no numbers to measure` |
| `WgpuEncoder` | DeviceError | `the encoder cannot be created: <reason>` |
| `WgpuEncoderFinished` | InvalidOperation | `the encoder was finished; open another`, or `the encoder was finished already` |
| `WgpuEntryPointAmbiguous` | InvalidArgument | `the shader has several <stage> entry points; name one with -Entry` |
| `WgpuFeatureExperimental` | InvalidArgument | `<name> is experimental; add -Experimental to request it` |
| `WgpuFeatureUnsupported` | InvalidArgument | `<adapter> does not support the feature <name>` |
| `WgpuFilter` | DeviceError | `the texture cannot be filtered: <reason>` |
| `WgpuFilterEdge` | InvalidArgument | `-FilterEdge says what a filter reads past the edge of the texture, and no filter was given` |
| `WgpuFinish` | DeviceError | `the recorded commands are invalid: <reason>` |
| `WgpuGrayscale` | InvalidArgument | `-GrayscaleWeights picks the weights of -Grayscale: give -Grayscale`, or `grayscale weights are Rec709, Rec601 or three numbers for red, green and blue, not '<text>'`, or `grayscale weights are Rec709, Rec601 or three finite numbers for red, green and blue; <n> numbers were given` |
| `WgpuImage` | InvalidArgument or ObjectNotFound | `<path> cannot be read as an image: <reason>`, or `<path> cannot be read as an image: <width>x<height> pixels: <reason>`, or `<path> cannot be read as <what>: <reason>`, or `the image has no texels to write`, or `<path> names no file`, or `Cannot find path '<path>' because it does not exist.`, or `<path> cannot be read: <reason>`, or `give the file as -Path or -LiteralPath` |
| `WgpuImageExtension` | InvalidArgument | `<path> names no image format the module writes: <reason>` |
| `WgpuImageFormat` | InvalidArgument | `an image file's pixels do not load straight into <format>`, or `a <format> texture decodes into <decoded format>, which has no image form; images come from the unsigned norm and float formats, and a .dds or .ktx2 file holds the texture whole`, or `a <format> texture has no image form; images come from the unsigned norm and float formats, and a .dds or .ktx2 file holds the texture whole`, or `-Format, -MipLevels, -MipFilter, -MipEdge, -KeepCoverage, -NormalMips, -TwoChannel, -Quality, -Effort, -Zstd and -Cube apply to .dds and .ktx2 files`, or `<name> is not an image format the image crate writes` |
| `WgpuImageShape` | InvalidArgument | `the texels do not make a <width>x<height> <color type> image` |
| `WgpuImageWrite` | WriteError | `<path> cannot be written: <reason>` |
| `WgpuImmediates` | InvalidArgument | `<n> bytes of immediate data were given and the entry point reads <immediate size>` |
| `WgpuIndexType` | InvalidArgument | `an index buffer holds UInt16 or UInt32, not <type>` |
| `WgpuJoin` | InvalidArgument | `<parameter> takes a texture, a texture and a channel letter, or a number, and <what was given> is none of those`, or `<parameter>'s channel letter is r, g, b or a, not '<letter>'`, or `<parameter> is a finite number, not <number>`, or `Join-WgpuTexture takes at least one texture among -Red, -Green, -Blue and -Alpha`, or `Join-WgpuTexture joins 2D textures of one layer, and <texture> is not one` |
| `WgpuKeepCoverage` | InvalidArgument | `-KeepCoverage is the alpha an alpha test passes at, above 0 and at most 1, not <r>` |
| `WgpuKtx2` | InvalidArgument | `a KTX2 file of <n> bytes is larger than Basis Universal reads: <reason>`, or `Basis Universal cannot read the KTX2 file's supercompressed data: <what Basis Universal printed>`, or `Basis Universal cannot transcode level <level> of the KTX2 file to the format chosen`, or `Basis Universal could not transcode level <level>, layer <layer>, face <face> of the KTX2 file: <what Basis Universal printed>`, or `a zstd level of the KTX2 file cannot be decompressed: <reason>`, or `a zstd level of the KTX2 file holds <n> bytes where its index records <size>`, or `a level cannot be compressed with zstd: <reason>` |
| `WgpuLayer` | InvalidArgument | `layer <layer> is past the texture's <layers>`, or `-Layout and -Panorama write all six faces into the image; -Layer picks one layer of a texture saved as it is`, or `a DDS or KTX2 file holds every layer; -Layer applies to other image files` |
| `WgpuLayout` | DeviceError | `the shader's bindings cannot be laid out: <reason>` |
| `WgpuLoader` | DeviceError or InvalidArgument | `<library> cannot be loaded and pinned: <reason>`, or `dxcompiler.dll was staged at <to string lossy>, which is not text, so wgpu cannot load it` |
| `WgpuMap` | DeviceError | `the mapped buffer cannot be read: <reason>`, or `the buffer cannot be read back: <reason>` |
| `WgpuMapUnused` | InvalidArgument | `the expression does not read <name>` |
| `WgpuMeasure` | InvalidArgument | `-Range takes two numbers, the low end and then the high end, finite, the first below the second`, or `-Bins is at least 1`, or `a histogram of <bins> bins cannot be held in memory: <reason>` |
| `WgpuMemberNotNumbers` | InvalidArgument | `member <name> of <type name> is not numbers; pack it by hand into a buffer` |
| `WgpuMemberSize` | InvalidArgument | `member <name> takes numbers in groups of <per element>; <n> were given`, or `member <name> takes <n> numbers; <n> were given` |
| `WgpuMipEdge` | InvalidArgument | `a mip level has no border color: the edge is Clamp, Wrap or Mirror, or ClampToEdge, Repeat or MirrorRepeat`, or `<filter> reads only the texels each new texel covers and never past an edge, so the <edge> edge changes nothing; Triangle, Kaiser, Mitchell and Lanczos read past one`, or `<name> is not a mip edge: the edge is Clamp, Wrap or Mirror, or ClampToEdge, Repeat or MirrorRepeat` |
| `WgpuMipFilter` | InvalidArgument | `-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how the mip levels -MipLevels adds and a transform renders again are made, and the call gives neither: give -MipLevels`, or `-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how the levels below level 0 are made, and -MipLevels 1 keeps level 0 alone`, or `a filter, an edge, a coverage and how normals stay unit length choose how the levels below level 0 are made, and GenerateMips(1) keeps level 0 alone`, or `-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how a texture's mip levels are rendered again, and this one has level 0 alone`, or `-MipFilter, -MipEdge and -NormalMips choose how the normal map's mip levels are rendered, and the height map has level 0 alone` |
| `WgpuMipLevels` | DeviceError or InvalidArgument | `mip levels are rendered in <format>, which this device does not render into`, or `the mip levels cannot be rendered: <reason>`, or `the cube has mip levels 0 to <last level>, not <level>`, or `faces <size> texels across have <levels> mip levels down to 1x1, not <n>`, or `a <width>x<height> texture has <levels> mip levels down to 1x1, not <n>`, or `<name> takes a full mip chain (-MipLevels 0) or a single level for an array, a cube map or an HDR format, not <count> levels`, or `GenerateMips() fills a texture's own mip levels, and this one has a single level: GenerateMips(0) returns a copy with the full chain` |
| `WgpuNoAdapter` | DeviceError or ObjectNotFound | `no adapter's name contains <text>; the adapters are: <adapters>`, or `no adapter is available: <reason>` |
| `WgpuNoBinding` | ObjectNotFound | `the shader uses no binding <group>:<binding>`, or `the shader uses no binding named <key>; it uses <bindings>` |
| `WgpuNoBuffer` | InvalidArgument | `no buffer was given`, or `a source and a destination are both needed` |
| `WgpuNoData` | InvalidArgument | `a buffer and data are both needed`, or `give the array x runs over with -InputObject or the pipeline`, or `no data was given` |
| `WgpuNoDevice` | InvalidOperation | `no device is open` |
| `WgpuNoDispatchSize` | InvalidArgument | `the shader binds no buffer to count invocations from; give -Workgroups or -Invocations` |
| `WgpuNoEntryPoint` | ObjectNotFound | `the shader has no entry point named <name>`, or `the shader has no <stage> entry point` |
| `WgpuNoGroup` | ObjectNotFound | `the pipeline has no bind group <group>; it has <n>` |
| `WgpuNoMember` | InvalidArgument | `<type name> has no member <key>; its members are <names>` |
| `WgpuNoPipeline` | InvalidArgument | `a pipeline and resources are both needed` |
| `WgpuNormalMap` | DeviceError or InvalidArgument | `<format> has no <what the step needs>, so <parameter> would change nothing; Import-WgpuImage and Export-WgpuImage write the texels into a -Format that has it`, or `<filter> takes each channel's <smallest or largest> value on its own, which makes no normal of a normal's channels: give -NormalMips with Box, Triangle, Kaiser, Mitchell or Lanczos`, or `-HeightToNormal makes directions, which <format> would encode as sRGB colors: give its linear form, <linear format>`, or `-TwoChannel lays X and Y out for a format with two channels or a Basis Universal LDR format, and <format> is HDR`, or `-TwoChannel keeps X and Y apart for a format with two channels, such as Rg8Unorm or Bc5RgUnorm, or a Basis Universal LDR format, which takes X in red, green and blue and Y in alpha; <format> is neither`, or `-TwoChannel lays X and Y out for the -Format they are written in: give one with two channels, such as Bc5RgUnorm, or a Basis Universal LDR format`, or `a normal map stores directions, which <format> would encode as sRGB colors: give its linear form, <linear format>`, or `<parameter> is a finite number, the slope a height step of 1 between neighboring texels gives, not <strength>`, or `the normal map cannot be made: <reason>`, or `-ReconstructZ and -Normalize change normals a texture holds, and -HeightToNormal makes normals from heights, each with its Z and of length 1: give one or the other`, or `-HeightChannel, -NormalKernel, -HeightEdge and -NormalAlpha say how -HeightToNormal makes normals from heights: give -HeightToNormal`, or `normals are made from the heights of a 2D texture, or of each layer of a 2D array, and <reason>` |
| `WgpuNoShader` | InvalidArgument | `no shader was given`, or `give the shader as -Source, -Path, -LiteralPath or -SpirV` |
| `WgpuNoTarget` | InvalidArgument | `give -Width and -Height, or a -Target texture` |
| `WgpuNotCompute` | InvalidArgument | `<name> is a <stage> entry point, not a compute one` |
| `WgpuNoTexture` | InvalidArgument | `no texture was given`, or `a texture and data are both needed` |
| `WgpuNotFinite` | InvalidArgument | `<value> has no WGSL literal` |
| `WgpuNotNumeric` | InvalidArgument | `a string is not numeric data; give an array of numbers` |
| `WgpuNotStruct` | InvalidArgument | `<name> is <type name>, not a struct, so it takes an array rather than a hashtable` |
| `WgpuNotTimestamps` | InvalidOperation | `only a timestamp query set reads as nanoseconds` |
| `WgpuNoVertexInput` | InvalidArgument | `the vertex shader has no input named <name>` |
| `WgpuNoVertices` | InvalidArgument or ObjectNotFound | `no vertex buffer was given for <name>`, or `the vertex shader reads <inputs>; give them with -Vertex`, or `-Vertex gives no values for <name>` |
| `WgpuNoWgslType` | InvalidArgument | `<variable> is <type>, which WGSL has no type for; convert it to Int32, UInt32 or Float32 first`, or `Measure-WgpuArray reduces Float16, Float32, Float64, Int32, UInt32, Int64 and UInt64; <type> has no WGSL type` |
| `WgpuOtherDevice` | InvalidArgument | `the resource for <name> lives on another device (<device>); resources and pipelines must share one`, or `a buffer copy runs between buffers of one device`, or `Join-WgpuTexture joins textures of one device`, or `the command buffer of encoder '<label>' was recorded on another device (<device>)`, or `the buffer lives on another device than the encoder` |
| `WgpuOutFile` | InvalidArgument or WriteError | `-OutFile holds one shader, and <path> matches <n> files: <files>`, or `<path> names no file`, or `-OutFile holds one shader, and <path> holds one from earlier in this pipeline`, or `<path> cannot be written: <reason>` |
| `WgpuOutOfRange` | InvalidArgument | `<value> does not fit in <type>, which holds <min> to <max>` |
| `WgpuOverrides` | InvalidArgument | `the shader's override constants cannot be resolved: <reason>` |
| `WgpuPad` | InvalidArgument | `-Pad takes two numbers, the width and height of the canvas; <n> were given`, or `a pad color is three or four numbers, red, green, blue and alpha; <n> were given`, or `-PadColor fills the padding: give -Pad`, or `-Pad grows the canvas, and <width>x<height> is smaller than the <texture width>x<texture height> texture it pads` |
| `WgpuPanic` | DeviceError | `the submission panicked: <message>`, or `the task's work panicked: <message>` |
| `WgpuPassEnded` | InvalidOperation | `the pass has ended` |
| `WgpuPipeline` | DeviceError | `the compute pipeline cannot be built: <reason>` |
| `WgpuPoll` | DeviceError | `waiting for the device failed: <reason>`, or `waiting for the newest submission on another WARP device failed: <reason>` |
| `WgpuQuality` | InvalidArgument | `-Quality runs from 1 to 100, not <quality>`, or `<name> has no quality setting, as BasisUastcHdr4x4 and the BasisAstc formats have none; -Quality applies to the other Basis formats`, or `-Quality applies to a Basis Universal -Format with a quality setting` |
| `WgpuQuery` | DeviceError | `the timestamp queries cannot be made: <reason>` |
| `WgpuQueryResolve` | DeviceError | `the queries cannot be resolved: <reason>` |
| `WgpuQuerySet` | DeviceError | `the query set cannot be created: <reason>` |
| `WgpuRange` | InvalidArgument | `element <start> is past the end of a buffer of <length> elements`, or `elements <start> to <start + count> run past the end of a buffer of <length> elements`, or `queries <first> to <first + count> run past the <count> the set holds` |
| `WgpuRead` | DeviceError | `the buffer cannot be read back: <reason>` |
| `WgpuRecordClear` | InvalidArgument | `record draws into a pass the caller began, whose load decides what the target holds first: give the draw a clear of None`, or `record draws into a pass the caller began, whose load decides what its depth target holds first: give the draw a clear_depth and a clear_stencil of None` |
| `WgpuRecordTimed` | InvalidArgument | `record dispatches into a pass the caller began, and timing needs queries around the whole pass: give a spec that is not timed` |
| `WgpuReduction` | DeviceError | `the reduction's partials cannot be read: <outcome>` |
| `WgpuRenderingIntent` | InvalidArgument | `-RenderingIntent chooses how colors convert from a file's color profile, and -IgnoreProfile converts none: give one or the other` |
| `WgpuRenderPassOption` | InvalidArgument | `a hashtable names every part of the pass by key, so BeginRenderPass takes it alone`, or `BeginRenderPass takes the keys Color, ClearColor, Depth, ClearDepth, ClearStencil, ResolveTarget, OcclusionQuerySet and Label, and <key> is none of them`, or `BeginRenderPass's hashtable names no Color, the texture or view the pass draws into`, or `OcclusionQuerySet is a Wgpu.QuerySet: <message>` |
| `WgpuRenderPipeline` | DeviceError | `the render pipeline cannot be built: <reason>` |
| `WgpuResize` | InvalidArgument | `Resize takes a width, a height or both; a side given as 0 keeps the aspect ratio`, or `-Filter applies with a new size: -Width, -Height, -Scale, -MaxSize or -PowerOfTwo`, or `give -Scale, or -Width and -Height, not both`, or `-Scale is a number above 0, not <scale>`, or `a width, height or size limit is at least 1 texel`, or `<side> texels has no power of two a texture's size holds above it`, or `give a new size, a crop or padding: -Width, -Height, -Scale, -MaxSize, -PowerOfTwo, -Crop, -CropMargin or -Pad` |
| `WgpuRotate` | InvalidArgument | `a rotation is 90, 180 or 270 degrees clockwise, not <degrees>` |
| `WgpuSampler` | DeviceError | `the sampler cannot be created: <reason>` |
| `WgpuSamples` | InvalidArgument | `-Samples is how many samples each texel takes, 1 or more` |
| `WgpuShader` | DeviceError | `the shader cannot be compiled on this device: <reason>` |
| `WgpuShaderFile` | ObjectNotFound | `<path> cannot be read: <reason>`, or `Cannot find path '<path>' because it does not exist.`, or `<path> names no file` |
| `WgpuShaderInvalid` | ParserError | `the shader is invalid: line <line>, column <column>: <message>`, or `the shader is invalid: <message>` |
| `WgpuSharpen` | InvalidArgument | `-Sharpen is how much of the detail to add, above 0, and <amount> is not`, or `-SharpenRadius is the standard deviation of the blur -Sharpen compares with, above 0, and <radius> is not`, or `-SharpenThreshold is a difference from 0 to 1, and <threshold> is not`, or `-SharpenRadius and -SharpenThreshold say how -Sharpen sharpens: give -Sharpen` |
| `WgpuSpirV` | InvalidArgument | `SPIR-V is a whole number of 4-byte words; <n> bytes is not`, or `<path> is not a whole number of 4-byte SPIR-V words`, or `SPIR-V is binary; give it as words or bytes with -SpirV, or as a .spv file` |
| `WgpuSrgb` | InvalidArgument | `-Srgb applies to an image file loaded without -Format or with a Basis Universal format; a GPU format says whether its color is sRGB, as Bc7RgbaUnormSrgb does` |
| `WgpuSsim` | DeviceError or InvalidArgument | `SSIM's pass cannot be built: <reason>`, or `a level <width> texels wide is more than SSIM's pass takes on <adapter>, whose storage bindings hold <n> bytes`, or `SSIM cannot be computed: <reason>` |
| `WgpuStage` | InvalidArgument | `a ray-tracing stage cannot be named here; give the entry point instead` |
| `WgpuStencil` | InvalidArgument | `a stencil test needs a depth target with a stencil aspect, and <what the depth target lacks>: give -DepthFormat Stencil8, Depth24PlusStencil8 or Depth32FloatStencil8` |
| `WgpuStencilReference` | InvalidArgument | `a draw's stencil_reference sets the value a stencil test compares with and Replace writes, and this program has no stencil test`, or `-StencilReference sets the value a stencil test compares with and Replace writes, and this pipeline has no stencil test` |
| `WgpuStopped` | OperationStopped | `the wait was stopped while the GPU was still working; the work goes on and finishes on its own` |
| `WgpuSubmit` | DeviceError | `the submission is invalid: <reason>` |
| `WgpuSubmitted` | InvalidOperation | `the command buffer of encoder '<label>' was already submitted; record the work again` |
| `WgpuSubmitThread` | DeviceError | `the device's submission thread has ended: <reason>` |
| `WgpuSwizzle` | InvalidArgument | `-Swizzle takes four letters of r, g, b, a, 0 and 1, one for each channel of the new texture, as 'bgra' or 'rrr1'; '<mask>' is not that` |
| `WgpuTaskThread` | DeviceError | `the task's thread cannot start: <reason>` |
| `WgpuTexelCount` | InvalidArgument | `mip <mip> of a <format> texture holds <expected> bytes; <n> were given` |
| `WgpuTexels` | InvalidArgument | `<format> texels are not read as colors on the CPU`, or `<format> has no texel layout`, or `<format> texels are not packed from colors on the CPU` |
| `WgpuTexture` | DeviceError | `the <multisampled or depth> target cannot be created: <reason>`, or `the texture cannot be created: <reason>` |
| `WgpuTextureDecode` | InvalidArgument | `the module decodes no <format> texels on the CPU`, or `mip level <mip> holds <n> bytes where its shape takes <size>`, or `<format> has no block layout`, or `<format> has no texel layout`, or `an ASTC block is 16 bytes: <reason>` |
| `WgpuTextureFile` | InvalidArgument | `a <format> texture has no layout a file can hold`, or `the file cannot be read as a DDS texture: <reason>`, or `the DDS file's format is one the module does not read`, or `the DDS file ends inside mip level <mip> of layer <layer>`, or `a DDS file cannot hold a <format> texture`, or `the DDS header cannot be made: <reason>`, or `the texture's <n> bytes do not fill the <size> a DDS file of its shape holds`, or `the DDS file cannot be written: <reason>`, or `the file cannot be read as a KTX2 texture: <reason>`, or `the KTX2 file names no format and is not in a Basis Universal format`, or `the KTX2 file's format, VkFormat <code>, is one the module does not read`, or `mip level <mip> of the KTX2 file cannot be inflated: <reason>`, or `the KTX2 file's supercompression, <scheme>, is one the module does not read for a GPU format`, or `mip level <mip> of the KTX2 file holds <n> bytes where its shape takes <expected>`, or `ASTC has no <width>x<height> block`, or `Basis Universal can transcode the KTX2 file to no format this device takes`, or `the KTX2 file is not in a Basis Universal format`, or `Basis Universal cannot transcode the KTX2 file into <format>`, or `a KTX2 file cannot hold a <format> texture`, or `the KTX2 data format descriptor for <format> cannot be made: <reason>` |
| `WgpuTextureFormat` | InvalidArgument | `this device cannot hold a <format> texture: it has no <features>`, or `a DDS file cannot hold <name>: give a .ktx2 path`, or `a <format> texture cannot be made: <reason>` |
| `WgpuTextureRead` | DeviceError or InvalidArgument | `the texture cannot be read in a shader: <reason>`, or `<format> has neither depth nor stencil to read`, or `<format> has no texel layout`, or `the module reads no multisampled <format> texels`, or `the module reads no <format> planes`, or `the module reads no <format> texels`, or `the texture cannot be read back: <reason>` |
| `WgpuTextureUsage` | InvalidArgument | `<the reference or the texture> needs <usages> among its usages <what it needs them for>`, or `<what the call does>: the texture needs <usages> among its usages`, or `<format> is a depth or stencil format, which cannot be a storage binding` |
| `WgpuTextureView` | DeviceError | `the view cannot be created: <reason>` |
| `WgpuTextureWrite` | DeviceError | `the texture cannot be written: <reason>` |
| `WgpuTooManyWorkgroups` | InvalidArgument | `<n> workgroups in <axis> is more than the device's <limit> per dimension; spread them over y and z` |
| `WgpuTracePath` | InvalidArgument | `<path> names no folder` |
| `WgpuTransform` | DeviceError or InvalidArgument | `-Anchor places a crop to a size or the padding: give -Crop with a width and height, or -Pad`, or `a cube map is not flipped or rotated, as its faces would no longer meet at their edges`, or `a cube map's faces stay square, and <a crop to, a resize to or a canvas of> <width>x<height> is not`, or `a 1D texture stays one texel high, and <parameter> makes it <height> texels high`, or `a 1D texture stays one texel high, so it is not rotated by 90 or 270 degrees`, or `the texture cannot be transformed: <reason>`, or `the brightest texel's luminance came back as <n> bytes: <reason>`, or `the module decodes no <format> texels`, or `a transform reads texels as floating-point colors, and <format> holds integers, depth or stencil`, or `-Format names <format>, which holds integers, depth or stencil, and a transform writes floating-point colors`, or `give -Flip, -Rotate, -Format, a filter, -Blur, -Sharpen or -Edges, or a change to the channels or colors: -Swizzle, -ReconstructZ, -Normalize, -FlipGreen, -Premultiply, -Unpremultiply, -ColorSpace, -AssignColorSpace, -Grayscale, -Invert, -Brightness, -Contrast, -Gamma, -Exposure, -ToneMap, -ColorKey, -AlphaThreshold or -FixAlphaBorder` |
| `WgpuTranslate` | InvalidArgument | `the shader cannot be written as <language>: <reason>` |
| `WgpuUncaptured` | DeviceError | `the device reported: <errors>` |
| `WgpuUnknownFeature` | InvalidArgument | `wgpu has no feature named <name>` |
| `WgpuUnknownFormat` | InvalidArgument | `<name> is not a Basis Universal format; they are <names>`, or `wgpu has no texture format named <name>; Get-WgpuTextureFormat lists them` |
| `WgpuVariableLength` | InvalidArgument | `<name> has <length> elements and x has <n>` |
| `WgpuVariableName` | InvalidArgument | `<name> cannot name a variable; use a WGSL identifier other than x and i` |
| `WgpuVersion` | InvalidArgument | `<text> is not a <language> version such as 1.5: <reason>`, or `HLSL has no shader model <major>.<minor> naga writes; it writes 5.0 to 6.9` |
| `WgpuVertexCount` | InvalidArgument | `the vertex shader reads <n> inputs and <n> were given`, or `the shader reads no vertex inputs; give -VertexCount` |
| `WgpuVertexInput` | InvalidArgument | `vertex input <name> is <type>, which no vertex format holds`, or `vertex input <name> has no vertex format` |
| `WgpuViewDimension` | InvalidArgument | `<name> is not a view dimension; give D1, D2, D2Array, Cube, CubeArray or D3` |
| `WgpuWrite` | DeviceError | `the buffer cannot be written: <reason>` |
| `WgpuWrongResource` | InvalidArgument | `<name> is a <kind> binding; it takes a <resource type>`, or `<type> is not a texture, a view or a sampler`, or `<name> is a <binding type> binding, which reads one layer, and the texture has <layers>; bind one through CreateView`, or `<name> is a <binding type> binding, which reads square layers, and the texture is <width>x<height>`, or `<name> is a <binding type> binding, which reads six layers, and the texture has <layers>; bind six through CreateView`, or `<name> is a <binding type> binding, which reads layers six at a time, and the texture has <layers>; bind a multiple of six through CreateView`, or `<name> is a <binding type> binding, which takes one mip level, and the texture has <levels>; bind one through CreateView` |
| `WgpuZstd` | InvalidArgument | `-Zstd applies to a .ktx2 file in a GPU format; a DDS file has no supercompression, and a Basis Universal format chooses its own` |

## Examples

A cmdlet's error, kept with `-ErrorVariable` while `-ErrorAction SilentlyContinue` keeps it off the screen:

```powershell
PS> Open-WgpuDevice -Adapter nosuchadapter -ErrorAction SilentlyContinue -ErrorVariable failed
PS> $failed[0].FullyQualifiedErrorId
WgpuNoAdapter,Pwrs.Modules.Wgpu.OpenWgpuDeviceCommand

PS> $failed[0].CategoryInfo.Category
ObjectNotFound
```

A method's error, and its id taken from the front of its message:

```powershell
PS> $buffer = New-WgpuBuffer -Data ([float[]](1, 2, 3))
PS> $buffer.ReadRange(5, 1)
MethodInvocationException: Exception calling "ReadRange" with "2" argument(s): "[WgpuRange] element 5 is past the end of a buffer of 3 elements"

PS> try { $buffer.ReadRange(5, 1) } catch { $_.Exception.InnerException.Message }
[WgpuRange] element 5 is past the end of a buffer of 3 elements

PS> try { $buffer.ReadRange(5, 1) } catch {
>>     if ($_.Exception.InnerException.Message -match '^\[(\w+)\]') { $Matches[1] }
>> }
WgpuRange
```

An asynchronous method checks its arguments before it returns, and throws as the method it runs does. An error after that faults the task it returned, whose `Exception` holds the same `Pwrs.PwrsException`. Here the device is destroyed before a read starts:

```powershell
PS> $gpu = Open-WgpuDevice
PS> $data = New-WgpuBuffer -Data ([float[]](1, 2, 3)) -Device $gpu
PS> $gpu.Destroy()
PS> $task = $data.ReadAsync($null)
PS> $task.Wait()
MethodInvocationException: Exception calling "Wait" with "0" argument(s): "One or more errors occurred. ([WgpuDeviceLost] the device was lost: Destroyed)"

PS> $task.Status
Faulted

PS> $task.Exception.InnerException.Message
[WgpuDeviceLost] the device was lost: Destroyed
```

## Errors that come from PowerShell

Some arguments never reach a cmdlet. PowerShell's parameter binder checks them first, and its records carry ids of its own:

| Id | When |
|---|---|
| `CannotConvertArgumentNoMessage` | A value that does not convert to the parameter's type, such as `-Backend Nope`, whose message lists the enum's names, or `-Length abc`. The command does not run. |
| `AmbiguousParameterSet` | Parameters from different parameter sets together, such as `-Data` with `-Length`. The command does not run. |
| `NamedParameterNotFound` | A parameter the cmdlet does not have. The command does not run. |
| `InputObjectNotBound` | A record on the pipeline that no parameter takes, such as a string piped to `Read-WgpuBuffer`. The command goes on with the next record. |

```powershell
PS> try { Open-WgpuDevice -Backend Nope } catch { $_.FullyQualifiedErrorId }
CannotConvertArgumentNoMessage,Pwrs.Modules.Wgpu.OpenWgpuDeviceCommand

PS> try { New-WgpuBuffer -Data 1 -Length 3 } catch { $_.FullyQualifiedErrorId }
AmbiguousParameterSet,Pwrs.Modules.Wgpu.NewWgpuBufferCommand

PS> 'text' | Read-WgpuBuffer -ErrorAction SilentlyContinue -ErrorVariable failed
PS> $failed[0].FullyQualifiedErrorId
InputObjectNotBound,Pwrs.Modules.Wgpu.ReadWgpuBufferCommand
```

## Errors that come from PWRS

PWRS, the runtime the module is built on, converts what a cmdlet is given into Rust values, and raises an id of its own for a value inside data that does not convert:

| Id | Category | When |
|---|---|---|
| `PwrsConversionError` | InvalidType | A value inside data does not convert to the number type it is read as, such as a string in an array of numbers, a hashtable where numbers go, or a negative number for an unsigned element type. The message names the type, as `cannot convert value to Double`. |

```powershell
PS> New-WgpuBuffer @('a', 'b') -ErrorAction SilentlyContinue -ErrorVariable failed
PS> $failed[0].FullyQualifiedErrorId
PwrsConversionError,Pwrs.Modules.Wgpu.NewWgpuBufferCommand

PS> $failed[0].CategoryInfo.Category
InvalidType
```
