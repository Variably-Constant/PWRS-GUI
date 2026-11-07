# ps-measure

Tools that measure how Photoshop 2026 composites a PSD's layers, so the
module's merge (`src/psd_merge.rs`, `src/psd_composite8.rs`) can match it.
Photoshop is the reference: a batch of documents that tabulate one feature
is written, Photoshop opens and saves each with its own composite, and a fit
script compares candidate arithmetic with what Photoshop stored.

Windows only: Photoshop is driven over COM.

## Setup

- A Python with `requirements.txt` installed; name it in `PS_MEASURE_PYTHON`
  for the corpus check, which takes `python` from PATH otherwise.
- A work folder outside the repository, named in `PS_MEASURE_WORK`, holding
  the corpora of `corpora.md`; batches, logs and probe output go there too.
- For the gate's module build, a `cargo-pwrs` that reads this repository's
  `Cargo.toml`, on PATH. In a bash PATH entry write the drive as `/c/`: a
  `C:` splits at its colon, and cargo then finds an older `cargo-pwrs`.
- Photoshop 2026 for the measuring steps. It holds a lot of memory: tell the
  machine's owner before it starts, and close it when the batches are done.

## Layout

| Folder | What it holds |
|---|---|
| `photoshop/` | `ps_run.ps1` runs a JSX over COM; `ps_resave.jsx` saves a batch folder's `written/*.psd` into its `photoshop/` with Photoshop's composite; `ps_lib.jsx` and `ps_stroke_batch.py` build shape layers with strokes |
| `writers/` | `ps_composite_batch.py` writes compositing batches with psd-tools; `ps_fills_write.py` writes the fills reference documents through `tools/psd_reference.py` |
| `fits/` | scripts that compare candidate arithmetic with Photoshop's composites |
| `corpus/` | `psd_check.sh` reads and merges the corpora with `psd-probe` and compares them with psd-tools and the stored composites; `psd_oracle.py`, `psd_layer_list.py` and `psd_resave_pick.py` inspect documents |
| `psd-probe/` | a crate that compiles the module's PSD sources by path and reads or merges files from the command line |
| `gate.sh` | the gate for a PSD change: unit tests, clippy, the module build, the Photoshop-document Pester tests, the wording check and the corpus check |

## A measuring batch

```bash
python -u -I -B writers/ps_composite_batch.py "$PS_MEASURE_WORK/out-composite2/written" modes
pwsh -NoProfile -File photoshop/ps_run.ps1 -Script photoshop/ps_resave.jsx -Arguments "$PS_MEASURE_WORK/out-composite2"
python -u -I -B fits/ps_blend_fit.py "$PS_MEASURE_WORK/out-composite2"
```

The fit scripts read batches by folder name under the folder they are given:

| Folder | `ps_composite_batch.py` mode | Holds |
|---|---|---|
| out-composite1 | (none) | normal at several opacities and fills, and two partly transparent layers |
| out-composite2 | `modes` | each separable mode's opaque table, source alpha 255 and 128 |
| out-composite3 | `nonsep` | the non-separable modes over random colors |
| out-composite4 | `clearmodes` | the modes over a backdrop at alpha 128 |
| out-composite5 | `randmodes` | random colors and alphas on both layers |
| out-composite6 | `fillmodes` | every mode at fill opacity 128 and 77 |
| out-composite7 | `fillspecial` | the eight fill-inside modes at six fills |
| out-composite8 | `randfill` | random layers at opacity 200 and fill 77 |
| out-composite9 | `fillalpha` | alpha tables of two fill-inside modes at fill |

`testset` writes the small documents under `tests/ps-render/composite`.
