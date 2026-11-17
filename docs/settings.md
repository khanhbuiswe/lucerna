# Settings

| File | Path |
|---|---|
| User settings | `~/.config/lucerna/settings.json` |
| User keybindings | `~/.config/lucerna/keybindings.json` |
| User themes | `~/.config/lucerna/themes/*.json` (Zed theme format) |
| Workspace settings | `<project>/.lucerna/settings.json` |
| Logs | `~/Library/Logs/Lucerna/` |

The settings use VS Code's names, for example:

```jsonc
{
  "editor.fontFamily": "Lilex",
  "editor.fontSize": 14,
  "editor.lineHeight": 1.6,
  "editor.wordWrap": "off",
  "editor.minimap.enabled": false,
  "window.autoDetectColorScheme": true,
  "workbench.preferredDarkColorTheme": "Lucerna Dark",
  "workbench.preferredLightColorTheme": "Lucerna Light",
  "files.exclude": { "**/dist": true },
  "[go]": { "editor.tabSize": 8 }
}
```

## Editing settings

| Setting | Default | Meaning |
|---|---|---|
| `editor.insertSpaces` | `true` | Tab inserts spaces |
| `editor.trimAutoWhitespace` | `true` | Remove auto-inserted indentation that was left unused |
| `editor.wordSeparators` | `` `~!@#$%^&*()-=+[{]}\|;:'",.<>/? `` | Characters that split words |
| `editor.cursorBlinking` | `"blink"` | Cursor blink style |
| `editor.cursorStyle` | `"line"` | Cursor shape |
| `editor.emptySelectionClipboard` | `true` | Copy or cut the whole line when nothing is selected |
| `editor.autoIndent` | `"full"` | Auto-indent on Enter: `none`, `keep`, `brackets`, `advanced` or `full` |
| `editor.detectIndentation` | `true` | Detect tabs or spaces and the tab size from the file |
| `editor.autoClosingBrackets` | `"languageDefined"` | Close brackets: `always`, `languageDefined`, `beforeWhitespace` or `never` |
| `editor.autoClosingQuotes` | `"languageDefined"` | Close quotes, same values |
| `editor.autoClosingOvertype` | `"auto"` | Type over a closer: `always`, `auto` (only ones the editor inserted) or `never` |
| `editor.autoClosingDelete` | `"auto"` | Delete the closer with its opener: `always`, `auto` or `never` |
| `editor.autoSurround` | `"languageDefined"` | Wrap a selection when typing a bracket or quote: `languageDefined`, `quotes`, `brackets` or `never` |
| `files.scanDepth` | `5` | Outside a git repository, folders this many levels deep load when you expand them. `0` scans everything. |
| `files.eol` | `"auto"` | Line ending for new text |
| `files.encoding` | `"utf8"` | Reserved: encoding for new files, not read yet (takes effect in a later release) |
| `files.defaultLanguage` | `""` | Language of a new Untitled file, by id or name (`"rust"`, `"Python"`). Empty or unknown: Plain Text |
| `files.insertFinalNewline` | `false` | Add a final newline on save |
| `files.trimFinalNewlines` | `false` | Remove extra final newlines on save |
| `files.trimTrailingWhitespace` | `false` | Remove trailing whitespace on save |
| `files.autoSave` | `"off"` | When to save automatically |
| `files.autoSaveDelay` | `1000` | Milliseconds before an auto save |
| `files.hotExit` | `"onExit"` | `"onExit"` keeps unsaved changes when Lucerna quits, without asking, and brings them back when the folder or file is reopened; `"off"` asks to save instead. User settings only |
| `files.saveConflictResolution` | `"askUser"` | What to do when the file changed on disk |
| `lucerna.externalChanges.autoMerge` | `true` | Merge an agent's non-overlapping changes into unsaved text |
