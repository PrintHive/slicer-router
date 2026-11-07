# Fixtures

Files named `real-*` were written by the real slicer. The rest of the test
data is built in code by `crates/slicer-router-core/tests/common/mod.rs`.

| File | Origin | Application tag | Printer profile |
| --- | --- | --- | --- |
| `real-bambu-studio-cli.3mf` | Bambu Studio 02.08.02.61, `BambuStudio --export-3mf cube.stl` | `BambuStudio-02.08.02.61` | none |
| `real-orcaslicer-cli.3mf` | OrcaSlicer 2.3.1, `OrcaSlicer --export-3mf cube.stl` | `BambuStudio-2.3.1` | none |
| `real-snapmaker-orca-cli.3mf` | Snapmaker Orca 2.4.0, `Snapmaker_Orca --export-3mf cube.stl` | `Snapmaker_Orca-2.4.0` | none |
| `real-prusaslicer-cli.3mf` | PrusaSlicer 2.9.5-beta2, `PrusaSlicer --export-3mf cube.stl` | `PrusaSlicer-2.9.5-beta2` | none (the CLI writes no `Slic3r_PE.config`) |
| `real-bambu-studio-p1s.3mf` | Bambu Studio 02.08.04.57 project, copied unchanged from PrintHive's test data | `BambuStudio-02.08.04.57` | `Bambu Lab P1S` |
| `real-snapmaker-orca-u1.3mf` | Snapmaker Orca 2.2.4 sliced project for a Snapmaker U1, trimmed to `3D/3dmodel.model` and `Metadata/project_settings.config` | `BambuStudio-2.2.4` | `Snapmaker U1` |

`cube.stl` is a 10 mm cube. The CLI exports carry no printer preset because the
slicers' CLIs would not accept a minimal preset pair on the machine that
generated these, so the printer-profile cases that need a preset come from the
two project files above or from the in-code builder.

The first two rows show why the `Application` tag alone cannot separate Bambu
Studio from OrcaSlicer: OrcaSlicer writes `BambuStudio-` plus its own three-part
version. The Snapmaker Orca rows show the same trap and its end: 2.2.4 writes
`BambuStudio-2.2.4`, and 2.4.0 writes `Snapmaker_Orca-2.4.0`.

The PrusaSlicer file's model `Title` and `Description` read `prusa-generic`, where
the other CLI exports leave them empty. Detection reads only `Application` and the
printer fields, so those two do not matter.
