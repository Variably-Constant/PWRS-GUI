# Test fonts

`NotoSans-Regular-subset.ttf` is the one face the SVG import's unit tests draw text in, so a drawing is the same on every platform whatever fonts the machine has. It is Noto Sans, `ofl/notosans/NotoSans[wdth,wght].ttf` at github.com/google/fonts 8b0a1d0, instanced at weight 400 and width 100 and subset to the glyphs of `Wgpu draws SVG text` with fontTools 4.68.0:

```text
fonttools varLib.instancer NotoSans[wdth,wght].ttf wght=400 wdth=100 -o NotoSans-Regular.ttf
pyftsubset NotoSans-Regular.ttf --text="Wgpu draws SVG text" --name-IDs=* --no-hinting --output-file=NotoSans-Regular-subset.ttf
```

It is under the SIL Open Font License 1.1, in `OFL.txt`, which names no reserved font name.
