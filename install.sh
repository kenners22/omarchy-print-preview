#!/bin/bash
# Print preview for Omarchy.
#
#   ./install.sh                  Ctrl+P in imv, Files right-click, Open with
#   ./install.sh --with-printer   also a "Preview" printer, so Ctrl+P → Print in
#                                 any app (Chromium, LibreOffice…) opens the preview
#   ./install.sh --undo           take everything back out
#
# Runs from wherever you cloned it (files are linked, so `git pull` updates it).
# Only the libraries and the optional printer need sudo.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)
BIN=~/.local/bin/print-preview
NAUTILUS=~/.local/share/nautilus-python/extensions/print-preview.py
DESKTOP=~/.local/share/applications/print-preview.desktop
IMV=~/.config/imv/config
HYPR=~/.config/hypr/hyprland.lua
UNITS=~/.config/systemd/user
BACKEND=/usr/lib/cups/backend/print-preview
SPOOL=/var/spool/print-preview
DEPS=(python-gobject python-cairo python-numpy poppler-glib zbar ghostscript)
STOCK_PRINT='<Ctrl+p> = exec lp "$imv_current_file"'
OUR_PRINT='<Ctrl+p> = exec print-preview "$imv_current_file" &'
BEGIN='-- >>> omarchy-print-preview'
END='-- <<< omarchy-print-preview'

say() { printf '\e[1m%s\e[0m\n' "$*"; }

remove_printer() {
  [[ -e $BACKEND ]] || lpstat -v Preview &>/dev/null || return 0
  systemctl --user disable --now print-preview.path 2>/dev/null || true
  rm -f "$UNITS/print-preview.path" "$UNITS/print-preview.service"
  systemctl --user daemon-reload
  if lpstat -d 2>/dev/null | grep -q ': Preview$'; then
    real=$(lpstat -e | grep -vx Preview | head -1 || true)
    [[ -n $real ]] && lpoptions -d "$real" >/dev/null
  fi
  sudo lpadmin -x Preview 2>/dev/null || true
  sudo rm -f "$BACKEND"
  sudo rm -rf "$SPOOL"
  say "Preview printer removed."
}

if [[ ${1:-} == --undo ]]; then
  remove_printer
  rm -f "$NAUTILUS" "$DESKTOP"
  [[ -L $BIN ]] && rm -f "$BIN"
  update-desktop-database ~/.local/share/applications 2>/dev/null || true
  # imv: back to Omarchy's own Ctrl+P (print straight away)
  [[ -f $IMV ]] && sed -i "s|^<Ctrl+p> = exec print-preview .*|$STOCK_PRINT|" "$IMV"
  [[ -f $HYPR ]] && sed -i "/^$BEGIN\$/,/^$END\$/d" "$HYPR" && { hyprctl reload >/dev/null 2>&1 || true; }
  say "Print preview removed. (Libraries stay installed.)"
  exit 0
fi

[[ -f $HYPR ]] || { echo "This needs Omarchy with Hyprland's Lua config ($HYPR)." >&2; exit 1; }

missing=()
for p in "${DEPS[@]}"; do pacman -Q "$p" &>/dev/null || missing+=("$p"); done
if (( ${#missing[@]} )); then
  say "Installing ${missing[*]} (needs sudo)…"
  sudo pacman -S --needed --noconfirm "${missing[@]}"
fi

chmod +x "$HERE/print-preview"
mkdir -p "$(dirname "$BIN")" "$(dirname "$NAUTILUS")" "$(dirname "$DESKTOP")"
ln -sfn "$HERE/print-preview" "$BIN"
ln -sfn "$HERE/nautilus-print-preview.py" "$NAUTILUS"
ln -sfn "$HERE/print-preview.desktop" "$DESKTOP"
update-desktop-database ~/.local/share/applications 2>/dev/null || true

# imv: Ctrl+P opens the preview instead of printing straight away
mkdir -p "$(dirname "$IMV")"
[[ -f $IMV ]] || cp "${OMARCHY_PATH:-/usr/share/omarchy}/config/imv/config" "$IMV" 2>/dev/null || printf '[binds]\n' >"$IMV"
cp "$IMV" "$IMV.bak.$(date +%s)"
if grep -q '^<Ctrl+p> =' "$IMV"; then
  sed -i "s|^<Ctrl+p> = .*|${OUR_PRINT//&/\\&}|" "$IMV"
else
  grep -q '^\[binds\]' "$IMV" || printf '\n[binds]\n' >>"$IMV"
  sed -i "/^\[binds\]/a $OUR_PRINT" "$IMV"
fi

# Hyprland: float it centred at a size that suits a page, and keep it solid
# (Omarchy applies its slight transparency before a tag can be dropped).
if ! grep -q "^$BEGIN\$" "$HYPR"; then
  cat >>"$HYPR" <<EOF

$BEGIN
o.window("print-preview", { float = true, center = true, size = { 760, 820 } })
o.window("print-preview", { tag = "-default-opacity", opacity = "1 1" })
$END
EOF
fi
hyprctl reload >/dev/null 2>&1 || true

if [[ ${1:-} == --with-printer ]]; then
  say "Adding the Preview printer (needs sudo)…"
  sudo install -m 0700 -o root -g root "$HERE/print-preview-backend" "$BACKEND"
  sudo install -d -m 0755 -o root -g root "$SPOOL"
  sudo install -d -m 0700 -o "$USER" -g "$(id -gn)" "$SPOOL/$USER"
  # -m raw: the job arrives exactly as the app sent it (a PDF from GTK apps and Chromium)
  sudo lpadmin -p Preview -E -v print-preview:/ -m raw \
    -D "Print preview" -L "Opens the preview; print to the real printer from there" 2>&1 | grep -vi deprecat || true
  mkdir -p "$UNITS"
  ln -sfn "$HERE/systemd/print-preview.path" "$UNITS/print-preview.path"
  ln -sfn "$HERE/systemd/print-preview.service" "$UNITS/print-preview.service"
  systemctl --user daemon-reload
  systemctl --user enable --now print-preview.path
  lpoptions -d Preview >/dev/null  # your default only; `lp -d <printer>` still prints direct
fi

pgrep -x nautilus >/dev/null && echo "Restart Files (nautilus -q) to get the right-click item."
say "Done. Open an image in imv and press Ctrl+P."
[[ ${1:-} == --with-printer ]] && say "Ctrl+P → Print in any app now opens the preview too."
echo "Undo: $HERE/install.sh --undo"
