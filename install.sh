#!/bin/bash
# Print preview for Omarchy.
#
#   ./install.sh                  build it, then: Ctrl+P in imv and in document viewers,
#                                 Files right-click, Open with, and the Preview button
#                                 in GTK print dialogs (instead of GNOME's)
#   ./install.sh --no-printer     remove the "Preview" printer an older version added
#   ./install.sh --undo           take everything back out, as it was before
#
# Runs from wherever you cloned it. The app is built here and linked, so after
# `git pull` just run ./install.sh again to rebuild it.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)
BIN=~/.local/bin/print-preview
BUILT=$HERE/target/release/print-preview
NAUTILUS=~/.local/share/nautilus-python/extensions/print-preview.py
DESKTOP=~/.local/share/applications/print-preview.desktop
IMV=~/.config/imv/config
HYPR=~/.config/hypr/hyprland.lua
UNITS=~/.config/systemd/user
STATE=${XDG_STATE_HOME:-~/.local/state}/omarchy-print-preview
BACKEND=/usr/lib/cups/backend/print-preview
SPOOL=/var/spool/print-preview
DEPS=(gtk4 poppler-glib ghostscript)
STOCK_PRINT='<Ctrl+p> = exec lp "$imv_current_file"'
OUR_PRINT='<Ctrl+p> = exec print-preview "$imv_current_file" &'
BEGIN='-- >>> omarchy-print-preview'
END='-- <<< omarchy-print-preview'
GTK_KEY=gtk-print-preview-command
GTK_CMD='print-preview --unlink-tempfile %f'
GTK_INIS=(~/.config/gtk-3.0/settings.ini ~/.config/gtk-4.0/settings.ini)

say() { printf '\e[1m%s\e[0m\n' "$*"; }
user_default() { sed -n 's/^Default \([^ ]*\).*/\1/p' ~/.cups/lpoptions 2>/dev/null | tail -1; }
ours() { lpstat -v Preview 2>/dev/null | grep -q 'print-preview:/$'; }

# GTK print dialogs' Preview button runs $GTK_KEY from settings.ini. Point it
# at this preview, saving any previous value once so --undo can put it back.
set_gtk_preview() {
  local ini saved
  for ini in "${GTK_INIS[@]}"; do
    saved=$STATE/$(basename "$(dirname "$ini")")-preview-command
    mkdir -p "$(dirname "$ini")" "$STATE"
    [[ -f $ini ]] || printf '[Settings]\n' >"$ini"
    grep -q '^\[Settings\]' "$ini" || printf '\n[Settings]\n' >>"$ini"
    if [[ ! -f $saved ]]; then  # empty = wasn't set; never save our own value as "previous"
      sed -n "s/^$GTK_KEY *= *//p" "$ini" | head -1 | grep -vxF "$GTK_CMD" >"$saved" || true
    fi
    sed -i "/^$GTK_KEY *=/d" "$ini"
    sed -i "/^\[Settings\]/a $GTK_KEY=$GTK_CMD" "$ini"
  done
}

restore_gtk_preview() {
  local ini saved prev
  for ini in "${GTK_INIS[@]}"; do
    saved=$STATE/$(basename "$(dirname "$ini")")-preview-command
    [[ -f $ini ]] && grep -qxF "$GTK_KEY=$GTK_CMD" "$ini" || { rm -f "$saved"; continue; }
    sed -i "/^$GTK_KEY *=/d" "$ini"
    prev=$(cat "$saved" 2>/dev/null || true)
    [[ -n $prev ]] && sed -i "/^\[Settings\]/a $GTK_KEY=$prev" "$ini"
    # drop a settings.ini we created that's now just its header
    [[ $(grep -cv '^\s*$' "$ini") == 1 ]] && grep -qx '\[Settings\]' "$ini" && rm -f "$ini"
    rm -f "$saved"
  done
}

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

# Older versions added a "Preview" printer (a CUPS backend, spool folder and a
# systemd watcher). Take all of it out, putting back your previous default printer.
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
  restore_gtk_preview
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
  say "Print preview removed. (Libraries and the build in $HERE/target stay.)"
  exit 0
fi

[[ -f $HYPR ]] || { echo "This needs Omarchy with Hyprland's Lua config ($HYPR)." >&2; exit 1; }

missing=()
for p in "${DEPS[@]}"; do pacman -Q "$p" &>/dev/null || missing+=("$p"); done
command -v cargo >/dev/null || missing+=(rust)
if (( ${#missing[@]} )); then
  say "Installing ${missing[*]} (needs sudo)…"
  sudo pacman -S --needed --noconfirm "${missing[@]}"
fi

say "Building print-preview…"
(cd "$HERE" && cargo build --release --locked --quiet)

mkdir -p "$STATE" "$(dirname "$BIN")" "$(dirname "$NAUTILUS")" "$(dirname "$DESKTOP")"
ln -sfn "$BUILT" "$BIN"
ln -sfn "$HERE/nautilus-print-preview.py" "$NAUTILUS"
ln -sfn "$HERE/print-preview.desktop" "$DESKTOP"

# GTK print dialogs (Document Viewer, LibreOffice, Files…): their Preview button
# opens this preview instead of GNOME's full-screen one. Print still prints.
set_gtk_preview
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
# Ctrl+P in a document viewer (Document Viewer/Evince, Papers) opens this
# preview on the open file, skipping the print dialog; every other window gets
# Ctrl+P as usual, passed on the way Omarchy's universal copy/paste does it.
# The block is rewritten on every run so updates reach existing installs.
if grep -qx -- "$BEGIN" "$HYPR" && grep -qx -- "$END" "$HYPR"; then
  sed -i "/^$BEGIN\$/,/^$END\$/d" "$HYPR"
fi
cat >>"$HYPR" <<'LUA'
-- >>> omarchy-print-preview
o.window("print-preview", { float = true, center = true, size = { 760, 820 } })
o.window("print-preview", { tag = "-default-opacity", opacity = "1 1" })
o.window("(org.gnome.Evince|org.gnome.Papers)", { tag = "+print-preview-viewer" })
hl.bind("CTRL + P", function()
  local window = hl.get_active_window()
  for _, tag in ipairs(window and window.tags or {}) do
    if tag:gsub("%*$", "") == "print-preview-viewer" then
      return hl.dispatch(hl.dsp.exec_cmd("print-preview --active-window"))
    end
  end
  hl.dispatch(hl.dsp.send_key_state({ mods = "CTRL", key = "P", state = "down" }))
  hl.timer(function()
    hl.dispatch(hl.dsp.send_key_state({ mods = "CTRL", key = "P", state = "up" }))
  end, { timeout = 50, type = "oneshot" })
end, { description = "Print (document viewers: print preview)" })
-- <<< omarchy-print-preview
LUA
hyprctl reload >/dev/null 2>&1 || true

if ours || [[ -e $BACKEND ]]; then
  say "The \"Preview\" printer from an older version is still set up; it's no longer used."
  echo "Remove it with: $HERE/install.sh --no-printer"
fi
pgrep -x nautilus >/dev/null && echo "Restart Files (nautilus -q) to get the right-click item."
say "Done. Open an image in imv, or a PDF in Document Viewer, and press Ctrl+P."
echo "Undo: $HERE/install.sh --undo"
