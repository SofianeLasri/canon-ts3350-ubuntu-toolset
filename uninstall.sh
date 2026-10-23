#!/usr/bin/env bash
# Uninstalls Canon TS3350 Toolset (the ~/.config/ts3350-toolset configuration is kept).
set -euo pipefail

APP_ID=io.github.sofianelasri.TS3350Toolset
QUEUE=Canon_TS3350
PREFIX=/usr/local
DATADIR=$PREFIX/share

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

info "Removing the virtual printer"
sudo lpadmin -x "$QUEUE" 2>/dev/null || true
# Remove the default printer set by install.sh (~/.cups/lpoptions).
if [[ -f ~/.cups/lpoptions ]]; then
    sed -i "/^Default $QUEUE\b/d" ~/.cups/lpoptions
fi

info "Removing files"
sudo rm -f \
    "$PREFIX/bin/ts3350-toolset" \
    /usr/lib/cups/backend/ts3350-duplex \
    "$DATADIR/applications/$APP_ID.desktop" \
    "$DATADIR/dbus-1/services/$APP_ID.service" \
    "$DATADIR/icons/hicolor/scalable/apps/$APP_ID.svg"
sudo rm -rf /usr/share/ppd/ts3350-toolset /var/spool/ts3350-duplex

sudo update-desktop-database -q "$DATADIR/applications" || true
sudo gtk-update-icon-cache -q -t -f "$DATADIR/icons/hicolor" || true
info "Uninstallation complete (preferences kept in ~/.config/ts3350-toolset)."
