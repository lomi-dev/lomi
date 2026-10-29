# Open files from the operating system

Installed Lomi builds support opening files from the system file manager:

- **macOS:** choose **Open With → Lomi** in Finder. The app declares itself as
  an alternate editor for text and data, including files without an extension.
- **Linux:** choose Lomi in the file manager's **Open With** menu. DEB, RPM and
  AppImage desktop entries advertise text, data and supported image MIME types.
- **Windows:** choose **Open with Lomi** in the file's context menu. On Windows
  11 this action may appear under **Show more options**. Both NSIS and MSI
  installers also register Lomi as an application in the system Open With
  chooser, including files without a recognized extension.

These registrations do not change existing default applications. Install or
update the packaged app to register the actions; running the frontend alone
does not register Lomi with the operating system.

The executable also accepts one or more file paths as arguments. Paths with
spaces must be quoted, and `--` allows names beginning with a dash:

```sh
lomi "/path/to/project/README.md" "/path/to/project/image.png"
lomi -- "-notes.txt"
```

Files open in an existing matching project and workspace. An already open
editor is selected with its unsaved buffer intact. Otherwise Lomi uses the
deepest matching open project, or opens the file's parent directory as a
project containing a file tab. Opening a file does not start a terminal.
Requests arriving during startup wait for session restoration; later requests
go to the existing application instance.

Lomi's existing editor limits still apply: text must be UTF-8 or BOM-marked
UTF-16 and at most 16 MiB. Supported images use the existing image preview and
its size limits. The system cannot determine whether arbitrary file contents
are editable, so unsupported binary content can still appear in the chooser;
Lomi displays the editor's normal unsupported-file error when opened.
