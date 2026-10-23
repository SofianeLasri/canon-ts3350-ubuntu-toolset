# Canon TS3350 Toolset

GNOME tools (Ubuntu 26.04) for the Canon Pixma TS3350, which has no official Linux driver:

- **Manual duplex printing**, like the Windows driver: a virtual printer named "Canon TS3350"
  appears in the print dialog of every application. When *Two-sided* is selected, a wizard
  prints the fronts, explains how to flip the stack, then prints the backs.
- **Multi-page scanning** with the scanner's real settings (colour, 75 to 600 dpi, document type,
  paper size or free area, sharpening, background reduction…), exported as **PDF**, **PNG**,
  **JPEG** or **AVIF** with adjustable quality and resolution.

The interface is available in English and French (it follows the system language).

## Installation

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev cups-client libglib2.0-bin
./install.sh
```

The script builds the project, installs the application and the CUPS backend, and creates the
`Canon_TS3350` queue linked to the printer's real queue (detected automatically, or set it with
`TARGET_QUEUE=<queue> ./install.sh`). `./uninstall.sh` removes everything except the preferences.

After installing, run **Duplex → Print a test page** once: two sheets are enough to check the
order and orientation of the backs, and the setting is corrected from your answers.

## How manual duplex works

The TS3350 reports `sides-supported = one-sided` and outputs sheets face up.

1. The virtual queue uses a PPD that exposes the `Duplex` option. CUPS runs `pdftopdf`
   (page ranges, n-up, copies), then hands the PDF to the `ts3350-duplex` backend.
2. The backend (running as root) forwards single-sided jobs straight to the real queue. Duplex jobs
   are dropped in `/var/spool/ts3350-duplex/<user>/`, then the application is woken up over
   D-Bus (`org.freedesktop.Application.Open`).
3. The wizard splits the PDF into two passes: odd pages, then even pages plus a blank page if
   needed. The backs are reversed and/or rotated by 180° depending on the binding and the
   calibration. Each pass is sent with `lp` and followed with `lpstat`.

## Architecture

| Crate | Role |
|---|---|
| `ts3350-core` | eSCL client, PDF/PNG/JPEG/AVIF export, duplex logic, CUPS, spool, preferences, i18n |
| `ts3350-backend` | `ts3350-duplex` CUPS backend |
| `ts3350-toolset` | GTK 4 / libadwaita application |

The scanner is driven directly over **eSCL** (HTTP), without SANE: `ScannerCapabilities`,
`ScanJobs`, `NextDocument`. It is discovered over mDNS (`_uscan._tcp`) or through an address set
in the preferences. The exported PDF embeds the scanner's JPEG images as they are, unless a lower
quality or resolution is chosen. There is no OCR.

## Translations

Source strings are written in English in the code (`i18n`, `i18n_f`, `ni18n_f` and `n_` from
`ts3350_core::i18n`). Translations live in standard gettext files under `po/`. `build.rs` compiles
them into the binary, so no `.mo` file or gettext tool is needed. The language follows `LANGUAGE`,
`LC_ALL`, `LC_MESSAGES` or `LANG`.

To add a language, copy `po/fr.po` to `po/<code>.po`, translate the `msgstr` entries and update the
`Plural-Forms` header. The `french_catalog_covers_every_source_message` test lists the French
messages that are missing or obsolete.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p ts3350-toolset                                # application
LANGUAGE=en cargo run -p ts3350-toolset                    # force English
cargo run -p ts3350-core --example escl-probe -- [url]     # scanner diagnostics
TS3350_DRY_RUN=/tmp/passes cargo run -p ts3350-toolset     # wizard without printing
```

In `TS3350_DRY_RUN` mode, the pass PDFs are copied into the given folder instead of being printed,
which allows testing the wizard end to end without paper.
