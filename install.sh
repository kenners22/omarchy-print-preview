#!/bin/bash
# Print preview for Omarchy.
#
#   ./install.sh                  Ctrl+P in imv, Files right-click, Open with
#   ./install.sh --with-printer   also a "Preview" printer, so Ctrl+P → Print in
#                                 any app (Chromium, LibreOffice…) opens the preview
#   ./install.sh --no-printer     remove just the "Preview" printer (keeps the rest)
#   ./install.sh --undo           take everything back out, as it was before
#
# Runs from wherever you cloned it. The app is linked, so `git pull` updates it;
# re-run ./install.sh after a pull to update the printer backend too (it's a
# root-owned copy).
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)
BIN=~/.local/bin/print-preview
NAUTILUS=~/.local/share/nautilus-python/extensions/print-preview.py
DESKTOP=~/.local/share/applications/print-preview.desktop
IMV=~/.config/imv/config
HYPR=~/.config/hypr/hyprland.lua
UNITS=~/.config/systemd/user
STATE=${XDG_STATE_HOME:-~/.local/state}/omarchy-print-preview
BACKEND=/usr/lib/cups/backend/print-preview
SPOOL=/var/spool/print-preview
DEPS=(python-gobject python-cairo python-numpy poppler-glib zbar ghostscript)
STOCK_PRINT='<Ctrl+p> = exec lp "$imv_current_file"'
OUR_PRINT='<Ctrl+p> = exec print-preview "$imv_current_file" &'
BEGIN='-- >>> omarchy-print-preview'
END='-- <<< omarchy-print-preview'

say() { printf '\e[1m%s\e[0m\n' "$*"; }
user_default() { sed -n 's/^Default \([^ ]*\).*/\1/p' ~/.cups/lpoptions 2>/dev/null | tail -1; }
ours() { lpstat -v Preview 2>/dev/null | grep -q 'print-preview:/$'; }

# Put a saved imv line back (or Omarchy's own if we never saved one).
restore_imv() {
  [[ -f $IMV ]] || return 0
  local saved=$STOCK_PRINT
  [[ -f $STATE/imv-ctrl-p ]] && saved=$(cat "$STATE/imv-ctrl-p")
  if [[ $saved == none ]]; then
    sed -i '/^<Ctrl+p> = exec print-preview /d' "$IMV"
  else
    local line=${saved//\\/\\\\}; line=${line//&/\\&}; line=${line//|/\\|}
    sed -i "s|^<Ctrl+p> = exec print-preview .*|$line|" "$IMV"
  fi
}

remove_printer() {
  ours || [[ -e $BACKEND ]] || return 0
  systemctl --user disable --now print-preview.path 2>/dev/null || true
  rm -f "$UNITS/print-preview.path" "$UNITS/print-preview.service"
  systemctl --user daemon-reload
  if [[ $(user_default) == Preview ]]; then
    prev=$(cat "$STATE/default-printer" 2>/dev/null || true)
    if [[ -n $prev ]] && lpstat -v "$prev" &>/dev/null; then
      lpoptions -d "$prev" >/dev/null
    else  # there was no personal default before: drop ours, the system one applies again
      sed -i '/^Default Preview\b/d' ~/.cups/lpoptions
    fi
  fi
  ours && sudo lpadmin -x Preview
  sudo rm -f "$BACKEND"
  sudo rm -rf "$SPOOL"
  rm -f "$STATE/default-printer" "$STATE/backend.sha256" "$STATE/ppd.sha256"
  say "Preview printer removed."
}

if [[ ${1:-} == --no-printer ]]; then
  remove_printer
  exit 0
fi

if [[ ${1:-} == --undo ]]; then
  remove_printer
  rm -f "$NAUTILUS" "$DESKTOP"
  [[ -L $BIN ]] && rm -f "$BIN"
  update-desktop-database ~/.local/share/applications 2>/dev/null || true
  restore_imv
  # Only our own block, and only if both markers are there (never "to the end of the file").
  if [[ -f $HYPR ]] && grep -qx -- "$BEGIN" "$HYPR" && grep -qx -- "$END" "$HYPR"; then
    sed -i "/^$BEGIN\$/,/^$END\$/d" "$HYPR"
    hyprctl reload >/dev/null 2>&1 || true
  fi
  rm -rf "$STATE"
  say "Print preview removed. (Libraries stay installed.)"
  exit 0
fi

[[ -f $HYPR ]] || { echo "This needs Omarchy with Hyprland's Lua config ($HYPR)." >&2; exit 1; }
if [[ ${1:-} == --with-printer ]] && lpstat -v Preview &>/dev/null && ! ours; then
  echo "There's already a printer called Preview that isn't this one; not touching it." >&2
  exit 1
fi

missing=()
for p in "${DEPS[@]}"; do pacman -Q "$p" &>/dev/null || missing+=("$p"); done
if (( ${#missing[@]} )); then
  say "Installing ${missing[*]} (needs sudo)…"
  sudo pacman -S --needed --noconfirm "${missing[@]}"
fi

mkdir -p "$STATE" "$(dirname "$BIN")" "$(dirname "$NAUTILUS")" "$(dirname "$DESKTOP")"
chmod +x "$HERE/print-preview"
ln -sfn "$HERE/print-preview" "$BIN"
ln -sfn "$HERE/nautilus-print-preview.py" "$NAUTILUS"
ln -sfn "$HERE/print-preview.desktop" "$DESKTOP"
update-desktop-database ~/.local/share/applications 2>/dev/null || true

# imv: Ctrl+P opens the preview instead of printing straight away.
# The line it replaces is saved once, so --undo can put it back exactly.
mkdir -p "$(dirname "$IMV")"
[[ -f $IMV ]] || cp "${OMARCHY_PATH:-/usr/share/omarchy}/config/imv/config" "$IMV" 2>/dev/null || printf '[binds]\n' >"$IMV"
if ! grep -q '^<Ctrl+p> = exec print-preview ' "$IMV"; then
  grep -m1 '^<Ctrl+p> =' "$IMV" >"$STATE/imv-ctrl-p" || echo none >"$STATE/imv-ctrl-p"
  cp "$IMV" "$IMV.bak.print-preview"
fi
if grep -q '^<Ctrl+p> =' "$IMV"; then
  sed -i "s|^<Ctrl+p> = .*|${OUR_PRINT//&/\\&}|" "$IMV"
else
  grep -q '^\[binds\]' "$IMV" || printf '\n[binds]\n' >>"$IMV"
  sed -i "/^\[binds\]/a $OUR_PRINT" "$IMV"
fi

# Hyprland: float it centred at a size that suits a page, and keep it solid
# (Omarchy applies its slight transparency before a tag can be dropped).
if ! grep -qx -- "$BEGIN" "$HYPR"; then
  cat >>"$HYPR" <<EOF

$BEGIN
o.window("print-preview", { float = true, center = true, size = { 760, 820 } })
o.window("print-preview", { tag = "-default-opacity", opacity = "1 1" })
$END
EOF
fi
hyprctl reload >/dev/null 2>&1 || true

if [[ ${1:-} == --with-printer ]] || ours; then
  # The installed copy is root-only, so remember what we installed instead of reading it back.
  want=$(sha256sum "$HERE/print-preview-backend" | cut -d' ' -f1)
  if [[ ! -e $BACKEND || $(cat "$STATE/backend.sha256" 2>/dev/null) != "$want" ]]; then
    say "Installing the Preview printer's backend (needs sudo)…"
    sudo install -m 0700 -o root -g root "$HERE/print-preview-backend" "$BACKEND"
    echo "$want" >"$STATE/backend.sha256"
  fi
  [[ -d $SPOOL ]] || sudo install -d -m 0755 -o root -g root "$SPOOL"
  [[ -d $SPOOL/$USER ]] || sudo install -d -m 0700 -o "$USER" -g "$(id -gn)" "$SPOOL/$USER"
  # A small PPD rather than a raw queue: apps like Chromium need paper sizes
  # before they'll print to it. Jobs still pass through untouched (see the PPD).
  ppd=$(sha256sum "$HERE/print-preview.ppd" | cut -d' ' -f1)
  if ! ours; then
    sudo lpadmin -p Preview -E -v print-preview:/ -P "$HERE/print-preview.ppd" \
      -D "Print preview" -L "Opens the preview; print to the real printer from there" 2>&1 | grep -vi deprecat || true
    echo "$ppd" >"$STATE/ppd.sha256"
  elif [[ $(cat "$STATE/ppd.sha256" 2>/dev/null) != "$ppd" ]]; then
    say "Updating the Preview printer's description (needs sudo)…"
    sudo lpadmin -p Preview -P "$HERE/print-preview.ppd" 2>&1 | grep -vi deprecat || true
    echo "$ppd" >"$STATE/ppd.sha256"
  fi
  mkdir -p "$UNITS"
  ln -sfn "$HERE/systemd/print-preview.path" "$UNITS/print-preview.path"
  ln -sfn "$HERE/systemd/print-preview.service" "$UNITS/print-preview.service"
  # (not `reenable`: on a linked unit its disable step deletes the link itself)
  rm -f "$UNITS/default.target.wants/print-preview.path"  # older installs hung it here
  systemctl --user daemon-reload
  systemctl --user enable print-preview.path 2>&1 | grep -v '^Created symlink' || true
  systemctl --user restart print-preview.path
  if [[ $(user_default) != Preview ]]; then
    user_default >"$STATE/default-printer"  # empty = none of my own; --undo restores either way
    lpoptions -d Preview >/dev/null         # my default only; `lp -d <printer>` still prints direct
  fi
fi

pgrep -x nautilus >/dev/null && echo "Restart Files (nautilus -q) to get the right-click item."
say "Done. Open an image in imv and press Ctrl+P."
ours && say "Ctrl+P → Print in any app opens the preview too."
echo "Undo: $HERE/install.sh --undo"
