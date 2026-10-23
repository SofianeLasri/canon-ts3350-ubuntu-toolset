#!/usr/bin/env bash
# Installs Canon TS3350 Toolset: GNOME application, CUPS backend and virtual printer.
#
#   ./install.sh                        detects the CUPS queue of the TS3350
#   TARGET_QUEUE=My_Queue ./install.sh  forces the real queue to use
set -euo pipefail

APP_ID=io.github.sofianelasri.TS3350Toolset
QUEUE=Canon_TS3350
PREFIX=/usr/local
BINDIR=$PREFIX/bin
DATADIR=$PREFIX/share
BACKEND=/usr/lib/cups/backend/ts3350-duplex
PPD=/usr/share/ppd/ts3350-toolset/ts3350-duplex.ppd
SPOOL=/var/spool/ts3350-duplex

cd "$(dirname "$(readlink -f "$0")")"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31mError:\033[0m %s\n' "$*" >&2; exit 1; }

if [[ $EUID -eq 0 ]]; then
    fail "run this script as a regular user: sudo is requested when needed."
fi

info "Checking dependencies"
missing=()
for pkg in libgtk-4-dev libadwaita-1-dev cups-client libglib2.0-bin; do
    dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q "install ok installed" || missing+=("$pkg")
done
if ((${#missing[@]})); then
    fail "missing packages: ${missing[*]}
    Install them with: sudo apt install ${missing[*]}"
fi
command -v cargo >/dev/null || fail "cargo not found (https://rustup.rs)."

info "Building (cargo build --release)"
cargo build --release --workspace

info "Looking for the Canon TS3300/TS3350 printer in CUPS"
target=${TARGET_QUEUE:-}
if [[ -z $target ]]; then
    target=$(LC_ALL=C lpstat -v 2>/dev/null \
        | sed -n 's/^device for \([^:]*\): \(.*\)$/\1 \2/p' \
        | grep -iE 'ts33|ts%2033' \
        | grep -v "^$QUEUE " \
        | grep -v ' ts3350-duplex:' \
        | head -n1 | cut -d' ' -f1 || true)
fi
if [[ -z $target ]]; then
    fail "no CUPS queue for the TS3350. Turn the printer on, add it in
    Settings → Printers, then run the script again (or TARGET_QUEUE=<queue> ./install.sh)."
fi
info "Real queue: $target"

info "Installing files (sudo)"
sudo install -Dm755 target/release/ts3350-toolset "$BINDIR/ts3350-toolset"
# Mode 0700 owned by root: CUPS then runs the backend as root, which it needs to write into the
# user's spool and to wake the application up over D-Bus.
sudo install -Dm700 -o root -g root target/release/ts3350-duplex "$BACKEND"
sudo install -Dm644 data/cups/ts3350-duplex.ppd "$PPD"
sudo install -Dm644 "data/$APP_ID.desktop" "$DATADIR/applications/$APP_ID.desktop"
sed "s|@BINDIR@|$BINDIR|" "data/$APP_ID.service.in" \
    | sudo install -Dm644 /dev/stdin "$DATADIR/dbus-1/services/$APP_ID.service"
sudo install -Dm644 "data/icons/hicolor/scalable/apps/$APP_ID.svg" \
    "$DATADIR/icons/hicolor/scalable/apps/$APP_ID.svg"
sudo install -d -m755 "$SPOOL"

info "Creating the “Canon TS3350” virtual printer"
sudo lpadmin -x "$QUEUE" 2>/dev/null || true
# PPD files are deprecated in CUPS 2.4 but remain the only way to expose the Duplex option.
sudo lpadmin -p "$QUEUE" \
    -D "Canon TS3350" \
    -L "Manual duplex through $target" \
    -v "ts3350-duplex:/$target" \
    -P "$PPD" \
    -o printer-error-policy=abort-job \
    -o printer-is-shared=false \
    -E 2>&1 | grep -v -i "deprecated" || true
LC_ALL=C lpstat -v "$QUEUE" >/dev/null 2>&1 || fail "creating the $QUEUE queue failed."

info "Updating desktop caches"
sudo update-desktop-database -q "$DATADIR/applications" || true
sudo gtk-update-icon-cache -q -t -f "$DATADIR/icons/hicolor" || true

read -r -p "Make “Canon TS3350” your default printer? [Y/n] " answer
if [[ ! $answer =~ ^[nN] ]]; then
    lpoptions -d "$QUEUE" >/dev/null
    info "Default printer: $QUEUE"
fi

cat <<EOF

Installation complete.
  • Application: “Canon TS3350” in the application grid (or ts3350-toolset).
  • Duplex: print to “Canon TS3350” and choose Two-sided: long edge or short edge in the
    print dialog. The wizard opens automatically.
  • Tip: run the calibration once (Duplex tab → Print a test page).
EOF
