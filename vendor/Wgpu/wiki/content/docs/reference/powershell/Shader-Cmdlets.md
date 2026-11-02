---
title: Shaders
weight: 40
---

The syntax, the parameters and the errors of `New-WgpuShader`, `Test-WgpuShader`, `Get-WgpuShaderInfo` and `ConvertTo-WgpuShader`. The examples are on [Shaders](Shaders.md).

### New-WgpuShader

Parses and validates a shader, independent of any device.

The shader is WGSL text, GLSL text (with -Language Glsl or a leading #version), a file, or SPIR-V words. It compiles on whichever device a pipeline is made on.

Alias: `New-GpuShader`

```powershell
New-WgpuShader [-Source] <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [-Label <string>] [<CommonParameters>]
New-WgpuShader -Path <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [-Label <string>] [<CommonParameters>]
New-WgpuShader -LiteralPath <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [-Label <string>] [<CommonParameters>]
New-WgpuShader -SpirV <Object> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [-Label <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Define` | IDictionary |  | Preprocessor definitions for GLSL, as name = value. |
| `-Label` | string |  | The shader's label. |
| `-Language` | [Wgpu.ShaderLanguage](Types.md#wgpushaderlanguage) |  | The language; inferred from the text or the file extension when omitted. |
| `-LiteralPath` | string | by name | A shader file named exactly as written, wildcard characters included; the files Get-ChildItem pipes bind here. Alias: `-PSPath`, `-LP`, `-FullName`. |
| `-Path` | string |  | A shader file: .wgsl, .spv, or GLSL (.comp, .vert, .frag, .glsl); a wildcard makes a shader of every file it matches. |
| `-Source` | string | by value | The shader text. |
| `-SpirV` | object |  | SPIR-V as byte[] or uint[]. |
| `-Stage` | [Wgpu.ShaderStage](Types.md#wgpushaderstage) |  | The stage of a GLSL shader; Compute when omitted. |

Writes [Wgpu.Shader](Types.md#wgpushader).

#### Errors

`WgpuNoShader`, `WgpuShaderFile`, `WgpuSpirV`, `WgpuStage` and `WgpuShaderInvalid`, each described in the [Error Reference](Error-Reference.md), as is PWRS's `PwrsConversionError` for SPIR-V that is not numbers.

### Test-WgpuShader

Validates a shader and writes whether it is valid, or with -Diagnostics every problem found, with its line and column.

Without -Device the shader is checked by naga as a device with every feature would check it; with -Device it is compiled on that device, against the features it has. A shader made with New-WgpuShader was validated as it was made, so it tests valid, and with -Device is compiled on that device.

Alias: `Test-GpuShader`

```powershell
Test-WgpuShader [-Source] <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [-Device <Device>] [-Diagnostics] [<CommonParameters>]
Test-WgpuShader -Path <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [-Device <Device>] [-Diagnostics] [<CommonParameters>]
Test-WgpuShader -LiteralPath <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [-Device <Device>] [-Diagnostics] [<CommonParameters>]
Test-WgpuShader -SpirV <Object> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [-Device <Device>] [-Diagnostics] [<CommonParameters>]
Test-WgpuShader [-Shader] <Shader> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [-Device <Device>] [-Diagnostics] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Define` | IDictionary |  | Preprocessor definitions for GLSL, as name = value. |
| `-Device` | [Wgpu.Device](Types.md#wgpudevice) |  | Compile the shader on this device as well. |
| `-Diagnostics` | switch |  | Write each problem found instead of $true or $false. |
| `-Language` | [Wgpu.ShaderLanguage](Types.md#wgpushaderlanguage) |  | The language; inferred from the text or the file extension when omitted. |
| `-LiteralPath` | string | by name | A shader file named exactly as written, wildcard characters included; the files Get-ChildItem pipes bind here. Alias: `-PSPath`, `-LP`, `-FullName`. |
| `-Path` | string |  | A shader file: .wgsl, .spv, or GLSL (.comp, .vert, .frag, .glsl); a wildcard checks every file it matches. |
| `-Shader` | [Wgpu.Shader](Types.md#wgpushader) | by value | A shader made with New-WgpuShader, given in place of text, by position or from the pipeline. |
| `-Source` | string | by value | The shader text. |
| `-SpirV` | object |  | SPIR-V as byte[] or uint[]. |
| `-Stage` | [Wgpu.ShaderStage](Types.md#wgpushaderstage) |  | The stage of a GLSL shader; Compute when omitted. |

Writes `bool` or [Wgpu.ShaderDiagnostic](Types.md#wgpushaderdiagnostic).

#### Errors

`WgpuNoShader`, `WgpuShaderFile`, `WgpuSpirV` and `WgpuStage`, each described in the [Error Reference](Error-Reference.md). A shader that is not valid is no error: the cmdlet writes `$false`, or with `-Diagnostics` its problems.

### Get-WgpuShaderInfo

Writes what a shader declares: its entry points with their stages and workgroup sizes, its resource bindings with their kinds and types, and its override constants.

Alias: `Get-GpuShaderInfo`

```powershell
Get-WgpuShaderInfo [-Source] <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [<CommonParameters>]
Get-WgpuShaderInfo -Path <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [<CommonParameters>]
Get-WgpuShaderInfo -LiteralPath <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [<CommonParameters>]
Get-WgpuShaderInfo -SpirV <Object> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [<CommonParameters>]
Get-WgpuShaderInfo [-Shader] <Shader> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Define <IDictionary>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Define` | IDictionary |  | Preprocessor definitions for GLSL, as name = value. |
| `-Language` | [Wgpu.ShaderLanguage](Types.md#wgpushaderlanguage) |  | The language; inferred from the text or the file extension when omitted. |
| `-LiteralPath` | string | by name | A shader file named exactly as written, wildcard characters included; the files Get-ChildItem pipes bind here. Alias: `-PSPath`, `-LP`, `-FullName`. |
| `-Path` | string |  | A shader file: .wgsl, .spv, or GLSL (.comp, .vert, .frag, .glsl); a wildcard describes every file it matches. |
| `-Shader` | [Wgpu.Shader](Types.md#wgpushader) | by value | A shader made with New-WgpuShader, given in place of text, by position or from the pipeline. |
| `-Source` | string | by value | The shader text. |
| `-SpirV` | object |  | SPIR-V as byte[] or uint[]. |
| `-Stage` | [Wgpu.ShaderStage](Types.md#wgpushaderstage) |  | The stage of a GLSL shader; Compute when omitted. |

Writes [Wgpu.ShaderInfo](Types.md#wgpushaderinfo).

#### Errors

`WgpuNoShader`, `WgpuShaderFile`, `WgpuSpirV`, `WgpuStage` and `WgpuShaderInvalid`, each described in the [Error Reference](Error-Reference.md).

### ConvertTo-WgpuShader

Translates a WGSL, GLSL or SPIR-V shader into WGSL, SPIR-V, GLSL, HLSL, MSL or a Graphviz graph of its intermediate form, with no GPU.

GLSL output is for one entry point; give -Entry or -Stage when the shader has several. SPIR-V comes out as uint[] words, or as a binary file with -OutFile.

Alias: `ConvertTo-GpuShader`

```powershell
ConvertTo-WgpuShader [-Source] <string> [-To] <ShaderTarget> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Entry <string>] [-Define <IDictionary>] [-GlslVersion <ushort>] [-GlslEs] [-ShaderModel <string>] [-MslVersion <string>] [-SpirVVersion <string>] [-OutFile <string>] [<CommonParameters>]
ConvertTo-WgpuShader [-To] <ShaderTarget> -Path <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Entry <string>] [-Define <IDictionary>] [-GlslVersion <ushort>] [-GlslEs] [-ShaderModel <string>] [-MslVersion <string>] [-SpirVVersion <string>] [-OutFile <string>] [<CommonParameters>]
ConvertTo-WgpuShader [-To] <ShaderTarget> -LiteralPath <string> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Entry <string>] [-Define <IDictionary>] [-GlslVersion <ushort>] [-GlslEs] [-ShaderModel <string>] [-MslVersion <string>] [-SpirVVersion <string>] [-OutFile <string>] [<CommonParameters>]
ConvertTo-WgpuShader [-To] <ShaderTarget> -SpirV <Object> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Entry <string>] [-Define <IDictionary>] [-GlslVersion <ushort>] [-GlslEs] [-ShaderModel <string>] [-MslVersion <string>] [-SpirVVersion <string>] [-OutFile <string>] [<CommonParameters>]
ConvertTo-WgpuShader [-Shader] <Shader> [-To] <ShaderTarget> [-Language <ShaderLanguage>] [-Stage <ShaderStage>] [-Entry <string>] [-Define <IDictionary>] [-GlslVersion <ushort>] [-GlslEs] [-ShaderModel <string>] [-MslVersion <string>] [-SpirVVersion <string>] [-OutFile <string>] [<CommonParameters>]
```

| Parameter | Type | Pipeline | Description |
|---|---|---|---|
| `-Define` | IDictionary |  | Preprocessor definitions for GLSL input, as name = value. |
| `-Entry` | string |  | The entry point to write; every entry point when omitted, where the target allows it. |
| `-GlslEs` | switch |  | Write OpenGL ES GLSL. |
| `-GlslVersion` | ushort |  | The GLSL version to write: 450 for desktop when omitted, 310 with -GlslEs. |
| `-Language` | [Wgpu.ShaderLanguage](Types.md#wgpushaderlanguage) |  | The language of the input; inferred from the text or the file extension when omitted. |
| `-LiteralPath` | string | by name | A shader file named exactly as written, wildcard characters included; the files Get-ChildItem pipes bind here. Alias: `-PSPath`, `-LP`, `-FullName`. |
| `-MslVersion` | string |  | The Metal Shading Language version; 2.4 when omitted. |
| `-OutFile` | string |  | Write the result to this file instead of the pipeline; SPIR-V as binary. |
| `-Path` | string |  | A shader file: .wgsl, .spv, or GLSL (.comp, .vert, .frag, .glsl); a wildcard translates every file it matches. |
| `-Shader` | [Wgpu.Shader](Types.md#wgpushader) | by value | A shader made with New-WgpuShader, given in place of text, by position or from the pipeline. |
| `-ShaderModel` | string |  | The HLSL shader model, 5.0 to 6.9; 6.0 when omitted. |
| `-Source` | string | by value | The shader text. |
| `-SpirV` | object |  | SPIR-V as byte[] or uint[]. |
| `-SpirVVersion` | string |  | The SPIR-V version, such as 1.5; naga's default when omitted. |
| `-Stage` | [Wgpu.ShaderStage](Types.md#wgpushaderstage) |  | The stage of a GLSL input, and the stage of the entry point to write. |
| `-To` | [Wgpu.ShaderTarget](Types.md#wgpushadertarget) |  | The language to write. |

Writes `string` or `uint[]`.

#### Errors

`WgpuNoShader`, `WgpuShaderFile`, `WgpuSpirV`, `WgpuStage`, `WgpuShaderInvalid`, `WgpuNoEntryPoint`, `WgpuEntryPointAmbiguous`, `WgpuOverrides`, `WgpuVersion`, `WgpuTranslate` and `WgpuOutFile`, each described in the [Error Reference](Error-Reference.md).
