# Test corpora

The corpus check (`corpus/psd_check.sh`) reads six folders of third-party
Photoshop documents from the work folder `PS_MEASURE_WORK`. They are not
in this repository; fetch each into a folder of the name below. Check each
source's terms before copying any file into the repository.

| Folder | Source | Terms |
|---|---|---|
| psd-real | psd-tools' test documents, `tests/psd_files` at github.com/psd-tools/psd-tools | MIT |
| psd-real2 | the same repository's subfolders (adjustments, blend-modes, effects and others); the `.urls` file beside each subfolder lists every file's raw.githubusercontent.com address | MIT |
| psd-real3 | ag-psd, github.com/Agamnentzar/ag-psd, the master archive unpacked (`ag-psd-master/test/read`, `test/write`) | MIT |
| psd-real4 | Krita's PSD test data, `plugins/impex/psd/tests/data` at invent.kde.org/graphics/krita | GPL, as Krita |
| psd-real5 | psd.rb, github.com/layervault/psd.rb, the master archive unpacked | MIT |
| psd-real6 | four UI kits from GitHub archives, unpacked as Bootflat.UI.Kit.PSD-master, Vapor-Kit-main, minimal-ui-kit-psd-master and psdrepo-master | each repository's own |

Files whose stored composite is not Photoshop's merge (version info's
hasRealMergedData false, or written by a library) are compared better
against a Photoshop re-save: `corpus/psd_resave_pick.py` copies the files a
summary lists into a batch folder, and `photoshop/ps_resave.jsx` saves them
again with Photoshop's own composite.
