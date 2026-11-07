# Slicer Router

Double-click a `.3mf` and it opens in the slicer that made it. Snapmaker
projects open in Snapmaker Orca, Bambu projects in Bambu Studio, Prusa projects
in PrusaSlicer, and OrcaSlicer projects in OrcaSlicer.

The project has three parts:

| Part | What it is |
| --- | --- |
| `crates/slicer-router-core` | Rust library: read a `.3mf`, decide which slicer owns it, find installed slicers, launch one. No GUI dependencies. |
| `crates/slicer-router-cli` | The `slicer-router` command. |
| `app/` | A small Tauri 2 app, "Slicer Router", that registers as the `.3mf` file handler on macOS and Windows. |

## How a file is routed

Detection reads three small entries from the `.3mf` zip and never loads the
model geometry, so a 100 MB file costs the same as a small one:

- `3D/3dmodel.model`, only the `<metadata name="Application">` tag at the top.
- `Metadata/project_settings.config` (Bambu Studio, OrcaSlicer, Snapmaker Orca), only the printer fields.
- `Metadata/Slic3r_PE.config` (PrusaSlicer), only the printer fields.

Rules run in this order and the first one that names a slicer wins. The
printer a project was prepared for outranks the program that saved it. Within
the printer rule, the printer model is checked first, then the preset name, then
the vendor, so a preset called `MK4S for Bambu filament` still goes to
PrusaSlicer. Within one field, slicers are tried in the order Bambu Studio,
Snapmaker Orca, PrusaSlicer.

| # | Rule name in `--json` | Fires when | Slicer |
| --- | --- | --- | --- |
| 1a | `printer-profile` | Printer model contains `Bambu`, `Snapmaker` or `Prusa`, or is exactly `MK3`, `MK3S`, `MK3.5`, `MK4`, `MK4S`, `MINI`, `XL`, `CORE One` or another Prusa model name | Bambu Studio, Snapmaker Orca or PrusaSlicer |
| 1b | `printer-profile` | No model matched, and the preset name (`printer_settings_id`) contains `Bambu`, `Snapmaker` or `Prusa` | Bambu Studio, Snapmaker Orca or PrusaSlicer |
| 1c | `printer-profile` | Neither matched, and the vendor contains `Bambu`, `Snapmaker` or `Prusa`. PrusaSlicer's `PRINTER_VENDOR_PRUSA3D` marker counts as the vendor | Bambu Studio, Snapmaker Orca or PrusaSlicer |
| 2 | `application-name` | `Application` is `BambuStudio-` followed by four numeric parts (for example `02.08.02.61` or `02.10.00.100`) | Bambu Studio |
| 2 | `application-bambu-fork` | `Application` is `BambuStudio-` followed by anything else (for example `BambuStudio-2.3.1`) | OrcaSlicer |
| 2 | `application-name` | `Application` starts with `OrcaSlicer-` | OrcaSlicer |
| 2 | `application-name` | `Application` starts with `Snapmaker_Orca-` | Snapmaker Orca |
| 2 | `application-name` | `Application` starts with `PrusaSlicer-` | PrusaSlicer |
| 3 | `prusa-config-file` | The archive has `Slic3r_PE.config` and nothing above matched | PrusaSlicer |
| - | `no-signal` | Nothing matched, for example a plain Printables or Thingiverse download | none, so your default slicer opens it |

Why the `BambuStudio-` tag needs two rows: OrcaSlicer 2.3 and Snapmaker Orca 2.2
write `BambuStudio-<their own version>` as the application so that Bambu Studio
accepts their files. Their versions have three parts. Bambu Studio's own
releases have four. Newer Snapmaker Orca builds write `Snapmaker_Orca-`.

A project with no matching printer and an `OrcaSlicer-` tag opens in OrcaSlicer.
A Bambu Lab P1S project saved by OrcaSlicer opens in Bambu Studio, because the
printer profile wins.

### Choosing the installed slicer

After detection, routing is a pure function of the verdict, the installed
slicers and your config:

1. The config override for the detected slicer, if that slicer is installed.
2. The detected slicer, if installed.
3. The configured default, if installed.
4. If none of those apply, a lone installed slicer is used. With none, the
   router reports that no slicer is installed. With several, it asks:
   - For a file with no signal, the answer becomes your default slicer.
   - For a file that belongs to a slicer you have not installed, the question
     says so ("Snapmaker Orca isn't installed"), and the answer is saved as an
     override for that slicer. Other files keep their own routing.

### Config

A JSON file, created when you pick a default:

| OS | Location |
| --- | --- |
| macOS | `~/Library/Application Support/Slicer Router/config.json` |
| Windows | `%APPDATA%\Slicer Router\config.json` |
| Other | `$XDG_CONFIG_HOME/slicer-router/config.json` |

Set `SLICER_ROUTER_CONFIG` to use another path.

```json
{
  "default_slicer": "orca-slicer",
  "overrides": { "bambu-studio": "orca-slicer" }
}
```

`overrides` maps a detected slicer to the slicer that should open its files
instead. Slicer ids are `bambu-studio`, `orca-slicer`, `snapmaker-orca` and
`prusa-slicer`.

A slicer id this version does not know is skipped with a warning instead of
failing the whole file, and other top-level keys are kept when the router saves.
Only a file that is not a JSON object is rejected. Choosing a default in the app
replaces such a file.

### How slicers are found

- **macOS.** Apps in `/Applications`, `~/Applications` and one folder level
  below them, matched by `CFBundleIdentifier`. PrusaSlicer's identifier ends in a
  `/` and the match ignores it. When two bundles share an identifier, the one with
  the shortest bundle name wins. Slicers are launched with `open -a <their .app>`,
  so the router never relaunches itself.
- **Windows.** The `App Paths` and `Uninstall` registry keys, then the usual
  `Program Files` folders. A path from the registry must be an `.exe` or a
  folder holding one, and `%VARIABLES%` in registry values are expanded. Slicers
  are started by running their executable with the file as the argument.

The file path is made absolute before launch. On macOS the router waits for
`open` to hand the file over, so a broken app bundle is reported as an error. On
Windows it returns as soon as the slicer process exists and does not see a later
failure inside the slicer.
- **Linux.** Not supported yet. The library compiles but finds nothing.

### How the app opens files

The app routes files before it starts any UI. Windows starts one process per
selected file, so each process detects its own file, launches the slicer and
exits. The app window and its single-instance handoff start only when a person is
needed: no file was given, a question has to be asked, or a launch failed.

macOS delivers opened files to the running app as an open-documents event, so the
app has to start first. That event arrives before the launch finishes (about
140 ms earlier on macOS 26), so when the app is ready it already knows whether
files came. It shows the settings window only for a direct launch, with no timer
and no window to take back.

## Install

There are no prebuilt releases yet. Build from source with a current stable Rust
toolchain.

```sh
# CLI
cargo install --path crates/slicer-router-cli

# App (needs the Tauri CLI once: cargo install tauri-cli --version "^2" --locked)
cd app/src-tauri
cargo tauri build --bundles app    # macOS, produces target/release/bundle/macos/Slicer Router.app
cargo tauri build --bundles nsis   # Windows, produces an installer under target/release/bundle/nsis/
```

On macOS copy `Slicer Router.app` to `/Applications`. The build is ad-hoc signed,
so the first launch may need right-click, Open.

## Make Slicer Router the default .3mf app

**macOS.** In Finder select any `.3mf`, press `Cmd+I`, choose Slicer Router
under Open with, then click Change All.

**Windows.** Settings, Apps, Default apps, search for `.3mf`, choose Slicer Router.

Then turn off "Associate .3mf files" in the preferences of Bambu Studio,
OrcaSlicer, Snapmaker Orca and PrusaSlicer (the label varies a little). Each of
them rewrites `HKCU\Software\Classes\.3mf` to itself every time it starts while
that option is on, and takes the association back from Slicer Router. Snapmaker
Orca does this in `GUI_App.cpp` at startup when `associate_3mf` is true.

The installer registers a project-owned file class, `PrintHive.SlicerRouter.3mf`,
instead of a generic "3MF Project" class, and lists it under the `.3mf`
`OpenWithProgids` key. Slicer Router therefore stays in the Default apps list even
after a slicer has taken the `.3mf` default. This lives in
`app/src-tauri/tauri.windows.conf.json` and `app/src-tauri/windows/hooks.nsh`, so
the macOS bundle is unchanged.

Launch Slicer Router directly to see the installed slicers, pick the slicer for
files with no origin, and read these steps again.

## CLI

```sh
slicer-router detect model.3mf          # verdict and evidence
slicer-router detect model.3mf --json   # same, as JSON
slicer-router open model.3mf            # route and launch
slicer-router open model.3mf --dry-run  # show the choice, launch nothing
slicer-router slicers                   # installed slicers
slicer-router default orca-slicer       # set the default; omit the id to show it
```

Example:

```text
$ slicer-router detect snapmaker-u1.3mf
slicer:        snapmaker-orca (Snapmaker Orca)
rule:          printer-profile
application:   BambuStudio-2.2.4
printer model: Snapmaker U1
```

## Library

```rust
use slicer_router_core::{detect, discover_installed, launch, route, Config, Route};

let verdict = detect("model.3mf".as_ref())?;
let installed = discover_installed();
let slicers: Vec<_> = installed.iter().map(|i| i.slicer).collect();
if let Route::Launch { slicer, .. } = route(&verdict, &slicers, &Config::default()) {
    let target = installed.iter().find(|i| i.slicer == slicer).unwrap();
    launch(target, "model.3mf".as_ref())?;
}
```

`detect` fails only when the file is not a readable zip. `classify` applies the
rules to evidence you already read. `Verdict::no_signal()` is what the app uses
for an unreadable file, so the chosen slicer shows its own error.

## Status

Verified on macOS: detection against real slicer output, discovery of the four
slicers, and the app opening files in the right slicer end to end.

Not yet run on Windows. The core crate type-checks and passes clippy for
`x86_64-pc-windows-msvc`, and the CI workflow runs the tests there, but registry
discovery, the Tauri app and the NSIS file association have not been exercised on
a Windows machine.

## License

MIT. See [LICENSE](LICENSE).
