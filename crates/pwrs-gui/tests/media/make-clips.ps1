# Makes the clips the media tests play into the folder this script is in:
# two seconds of a moving test picture with a 1 kHz tone, one clip for
# each container and codec a backend claims, and two with a track of
# subtitles of their own. The clips are made once and kept in the
# repository; run the script again only to change them, or with -Name to
# make the clips it names alone.
#
#   pwsh -NoProfile -File make-clips.ps1 [-FFmpeg <path to ffmpeg.exe>] [-Name <clip>,...]
#
# It needs an FFmpeg build with libx264, libx265, libvpx, libaom, libopus,
# libvorbis, libtheora, libmp3lame and libopencore-amrnb, such as the full
# build from gyan.dev. FFmpeg is a tool here only; nothing of it ships.
param([string] $FFmpeg = 'ffmpeg', [string[]] $Name = @())
$ErrorActionPreference = 'Stop'

$picture = @('-f', 'lavfi', '-i', 'testsrc2=size=160x120:rate=15:duration=2')
$tone = @('-f', 'lavfi', '-i', 'sine=frequency=1000:sample_rate=48000:duration=2')
# The cues of cues.srt as a track of the clip's own, after the picture and
# the tone.
$cues = @('-i', (Join-Path $PSScriptRoot 'cues.srt'), '-map', '0:v', '-map', '1:a', '-map', '2:s')
# Without the encoders' names and dates, the same FFmpeg makes the same
# bytes each time.
$exact = @('-map_metadata', '-1', '-fflags', '+bitexact', '-flags:v', '+bitexact', '-flags:a', '+bitexact')
$h264 = @('-c:v', 'libx264', '-preset', 'veryslow', '-crf', '30', '-pix_fmt', 'yuv420p')
$aac = @('-c:a', 'aac', '-b:a', '48k')

$clips = [ordered]@{
    'h264-aac.mp4'      = $picture + $tone + $h264 + $aac + '-movflags', '+faststart'
    'h264-aac-subs.mp4' = $picture + $tone + $cues + $h264 + $aac + '-c:s', 'mov_text', '-movflags', '+faststart'
    'vp9-opus-subs.mkv' = $picture + $tone + $cues + '-c:v', 'libvpx-vp9', '-b:v', '0', '-crf', '45', '-c:a', 'libopus', '-b:a', '32k', '-c:s', 'srt'
    'hevc-aac.mp4'      = $picture + $tone + '-c:v', 'libx265', '-crf', '32', '-pix_fmt', 'yuv420p', '-tag:v', 'hvc1', '-x265-params', 'log-level=error' + $aac + '-movflags', '+faststart'
    'h264-aac.mov'      = $picture + $tone + $h264 + $aac + '-f', 'mov'
    'h264-aac.mkv'      = $picture + $tone + $h264 + $aac
    'h264-aac.ts'       = $picture + $tone + $h264 + $aac + '-f', 'mpegts'
    'mpeg4-aac.3gp'     = $picture + $tone + '-c:v', 'mpeg4', '-q:v', '8' + $aac + '-ar', '48000', '-f', '3gp'
    'vp9-opus.webm'     = $picture + $tone + '-c:v', 'libvpx-vp9', '-b:v', '0', '-crf', '45', '-c:a', 'libopus', '-b:a', '32k'
    'vp8-vorbis.webm'   = $picture + $tone + '-c:v', 'libvpx', '-b:v', '150k', '-c:a', 'libvorbis', '-q:a', '1'
    'av1-opus.webm'     = $picture + $tone + '-c:v', 'libaom-av1', '-cpu-used', '8', '-b:v', '0', '-crf', '45', '-c:a', 'libopus', '-b:a', '32k'
    'theora-vorbis.ogv' = $picture + $tone + '-c:v', 'libtheora', '-q:v', '4', '-c:a', 'libvorbis', '-q:a', '1'
    'mjpeg-pcm.avi'     = $picture + $tone + '-c:v', 'mjpeg', '-q:v', '10', '-c:a', 'pcm_s16le', '-ar', '16000'
    'wmv2-wmav2.wmv'    = $picture + $tone + '-c:v', 'wmv2', '-b:v', '150k', '-c:a', 'wmav2', '-b:a', '48k'
    'tone.mp3'          = $tone + '-c:a', 'libmp3lame', '-b:a', '64k'
    'tone.m4a'          = $tone + $aac + '-movflags', '+faststart'
    'tone.aac'          = $tone + $aac + '-f', 'adts'
    'tone.wav'          = $tone + '-c:a', 'pcm_s16le', '-ar', '16000'
    'tone.flac'         = $tone + '-c:a', 'flac', '-ar', '16000'
    'tone.ogg'          = $tone + '-c:a', 'libvorbis', '-q:a', '1'
    'tone.opus'         = $tone + '-c:a', 'libopus', '-b:a', '32k'
    'tone.aiff'         = $tone + '-c:a', 'pcm_s16be', '-ar', '16000', '-f', 'aiff'
    'tone.caf'          = $tone + '-c:a', 'alac', '-ar', '16000', '-f', 'caf'
    'tone.wma'          = $tone + '-c:a', 'wmav2', '-b:a', '48k'
    'tone.ac3'          = $tone + '-c:a', 'ac3', '-b:a', '64k'
    'tone.amr'          = $tone + '-c:a', 'libopencore_amrnb', '-ar', '8000', '-ac', '1', '-b:a', '12.2k'
}

$unknown = @($Name | Where-Object { -not $clips.Contains($_) })
if ($unknown) { throw "make-clips.ps1 makes no clip named $($unknown -join ', ')" }
$total = 0
foreach ($clip in @($clips.Keys)) {
    if ($Name.Count -gt 0 -and $Name -notcontains $clip) { continue }
    $out = Join-Path $PSScriptRoot $clip
    & $FFmpeg -hide_banner -loglevel error -y @($clips[$clip]) @exact $out
    if ($LASTEXITCODE -ne 0) { throw "FFmpeg could not make $clip (exit code $LASTEXITCODE)" }
    $size = (Get-Item -LiteralPath $out).Length
    $total += $size
    '{0,-20} {1,9:N0} bytes' -f $clip, $size
}
'{0,-20} {1,9:N0} bytes' -f 'all', $total
