import shutil

from gi import require_version

require_version("Nautilus", "4.1")

from gi.repository import GObject, Gio, Nautilus

PRINTABLE = ("image/", "application/pdf")


class PrintPreviewAction(GObject.GObject, Nautilus.MenuProvider):
    """Right-click → "Print preview…" for images and PDFs."""

    def get_file_items(self, *args):
        files = args[0] if len(args) == 1 else args[1]
        command = shutil.which("print-preview")
        paths = []
        for file in files:
            location = file.get_location()
            path = location.get_path() if location else None
            if not path or not (file.get_mime_type() or "").startswith(PRINTABLE):
                return []
            paths.append(path)
        if not paths or not command:
            return []
        item = Nautilus.MenuItem(name="PrintPreview::open", label="Print preview…", icon="document-print-preview")
        item.connect("activate", lambda *_: Gio.Subprocess.new([command, *paths], Gio.SubprocessFlags.NONE))
        return [item]
