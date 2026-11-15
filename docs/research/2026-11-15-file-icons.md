# Icon file có màu và icon trên tab: VS Code và Zed

Ngày: 2026-11-15. Phục vụ spec icon màu trong Explorer và icon file trên tab editor.

## VS Code

- **Icon theme mặc định là Seti** (`workbench.iconTheme: "vs-seti"`). VS Code có sẵn hai theme: Seti và Minimal; chọn `None` để tắt icon. Đổi theme bằng lệnh **Preferences: File Icon Theme** (`workbench.action.selectIconTheme`), có xem trước bằng phím mũi tên.
- **Seti** nằm ở `microsoft/vscode/extensions/theme-seti` (MIT). Icon lấy từ [`jesseweed/seti-ui`](https://github.com/jesseweed/seti-ui) (MIT), đóng thành font `seti.woff`. File `vs-seti-icon-theme.json` có:
  - 383 `iconDefinitions`, mỗi cái là một glyph (`fontCharacter`) và **một màu** (`fontColor`). Mỗi glyph có bản `_light` cho theme sáng; script của VS Code chỉ nhân mỗi kênh màu tối với 0,9 (ví dụ `_R` `#519aba`, `_R_light` `#498ba7`).
  - Ánh xạ `fileExtensions` (238), `fileNames` (101), `languageIds` (83); khóa `light` ghi đè cho theme sáng.
  - **Không có icon thư mục**: Explorer với Seti chỉ có chevron, thư mục không có icon (đúng như ảnh người dùng gửi).
- Bản thân `seti-ui` có 169 SVG đơn sắc, viewBox `0 0 32 32`, tổng khoảng 175 KB. Màu theo loại file nằm ở `styles/components/icons/mapping.less` (`.icon-set(".ts", "typescript", @blue)`), dùng khoảng chục màu có tên (`@blue`, `@red`, `@green`, `@yellow`, `@orange`, `@purple`, `@pink`, `@white`, `@grey`…).
- **Icon trên tab:** tab hiện icon của icon theme đang dùng. Setting `workbench.editor.showIcons` (mặc định `true`) bật/tắt icon trên tab. Breadcrumbs có `breadcrumbs.icons` riêng (Lucerna đã có).

## Zed

- **Icon theme mặc định là "Zed (Default)"**, setting `icon_theme` (chuỗi, hoặc object `mode`/`dark`/`light`). Icon mặc định **đơn sắc**, tô bằng màu `icon.muted` của theme. Ai muốn màu thì cài extension, ví dụ "Colored Zed Icons Theme", Material Icon Theme, Catppuccin Icons.
- Icon theme từ extension theo schema `icon_themes/v0.3.0` (`file_stems`, `file_suffixes`, `file_icons`, `directory_icons`, `named_directory_icons`, `chevron_icons`). Icon SVG từ file ngoài được vẽ dạng ảnh nên **giữ màu gốc** (Zed #23195 thêm Icon từ file ngoài).
- **Icon trên tab:** `tabs.file_icons`, mặc định `false`. `tabs.git_status` (mặc định `false`) tô màu tên tab theo git. `tabs.show_diagnostics` chỉ chạy khi `file_icons` bật. Project panel có `file_icons` và `folder_icons` (mặc định `true`).
- Icon tab nằm bên trái tên, cùng cỡ với icon trong project panel, cách tên một khoảng nhỏ.

## Giống và khác

| | VS Code | Zed |
|---|---|---|
| Theme mặc định | Seti, có màu theo loại file | Zed (Default), đơn sắc |
| Icon thư mục | Seti: không có | Có (folder/folder-open) |
| Icon trên tab | Bật mặc định, `workbench.editor.showIcons` | Tắt mặc định, `tabs.file_icons` |
| Theme sáng/tối | Seti có bộ màu `light` riêng | `icon_theme` object `light`/`dark` |

## Lucerna hiện tại

- `lucerna-theme/src/icon_theme.rs`: theme dựng sẵn "Lucide" (đơn sắc, 10 loại: code, json, markdown, config, shell, image, lock, git, license, default) và icon theme từ extension Zed (`IconThemeRegistry`, `ResolvedIcons`). Theme extension thiếu icon thì rơi về Lucide.
- `workbench/src/ui.rs::file_icon_sized`: icon Lucide vẽ bằng `svg()` (GPUI dùng alpha làm mặt nạ rồi tô `text_color`), icon extension vẽ bằng `img()` nên giữ màu.
- Explorer tô mọi icon Lucide bằng `ui.icon_muted`.
- Tab (`editor_groups/view.rs`): tab đặc biệt (Search, Extensions, Git…) đã có icon 14 px; tab file (`TabItem::File`) chưa có. `ui::tab` đã có ô `icon`.

## Lucerna chọn gì

Xem spec `docs/superpowers/specs/2026-11-15-file-icons-design.md`. Theo nguyên tắc: setting và hành vi theo VS Code (`workbench.editor.showIcons`, mặc định bật), giao diện theo Zed (vị trí, cỡ icon, khoảng cách). Riêng màu icon mặc định thì lệch khỏi Zed theo yêu cầu người dùng.
