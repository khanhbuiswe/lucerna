# Đóng gói Lucerna thành macOS app bundle

Vấn đề: chạy `luc` như một file thực thi trần thì menu bar hiện "luc" và Dock hiện icon mặc định. macOS lấy tên app từ `CFBundleName` trong `Info.plist` của bundle chứa file thực thi; `cx.set_menus` không đổi được tên này.

## VS Code (xem bản cài ở `/Applications/Visual Studio Code.app`)

- `Info.plist`: `CFBundleName`/`CFBundleDisplayName` = Code, `CFBundleExecutable` = Code, `CFBundleIconFile` = Code.icns, `CFBundleIdentifier` = com.microsoft.VSCode, `LSMinimumSystemVersion` = 12.0, `NSHighResolutionCapable`, `LSApplicationCategoryType` = public.app-category.developer-tools.
- Lệnh `code` là shell script `Contents/Resources/app/bin/code`. Script lần theo symlink để tìm `.app`, rồi chạy chính binary Electron trong `Contents/MacOS` (chế độ Node) với `cli.js`; `cli.js` mở app và chuyển tham số qua IPC tới instance đang chạy.
- Lệnh "Shell Command: Install 'code' command in PATH" tạo symlink `/usr/local/bin/code` → script trên.

## Zed (xem bản cài ở `/Applications/Zed.app`)

- `Info.plist`: `CFBundleName`/`CFBundleDisplayName` = Zed, `CFBundleExecutable` = zed, `CFBundleIconFile` = Zed.icns, `CFBundleIdentifier` = dev.zed.Zed, `LSMinimumSystemVersion` = 12.0, `NSHighResolutionCapable`, category developer-tools.
- `Contents/MacOS` có ba binary: `zed` (app), `cli` và `git`. Lệnh `zed` là binary `cli`: nó tìm bundle (`Contents/MacOS/zed`), mở app và gửi đường dẫn qua IPC; app chạy một instance duy nhất. Lệnh "cli: install" tạo symlink tới `cli`.
- Zed dựng bundle bằng `cargo-bundle` (metadata `[package.metadata.bundle]` trong `crates/zed/Cargo.toml`) trong `script/bundle-mac`, rồi ký và notarize.

## Giống và khác

- Giống: cả hai ship `.app` với tên và icon trong `Info.plist`, và cài một lệnh CLI vào PATH bằng symlink tới một file nằm trong bundle.
- Khác: VS Code dùng script và chính binary app; Zed dùng binary CLI riêng. Cả hai cùng chuyển tham số tới một instance app duy nhất qua IPC.
- Icon của cả hai theo khung chuẩn của Apple: tile 824 px giữa canvas 1024 px.

## Lucerna chọn

- **Dựng bundle bằng `scripts/bundle-app.sh`**, không dùng `cargo-bundle`: không thêm dependency, chỉ dùng `sips`, `iconutil`, `plutil` và `codesign` có sẵn trên macOS. Icon render từ `assets/brand/app-icon.svg`, thu về tile 824/1024 như VS Code và Zed.
- **`Info.plist`** theo cùng bộ key với hai bên. Bundle ID là `dev.lucerna.Lucerna`, theo kiểu của Zed. `LSMinimumSystemVersion` là 13.0, đúng mức Lucerna hỗ trợ. Có thêm `NSPrincipalClass` = `NSApplication`: thiếu key này thì khi chạy trong bundle, AppKit ghi lỗi `getApplicationProperty: called with invalid property` lúc khởi động.
- **Lệnh `luc`** là shim kiểu VS Code: shell script `exec` thẳng binary `Contents/MacOS/luc`. Binary nằm trong bundle nên macOS dùng tên và icon của bundle. Chạy thẳng binary giữ nguyên cwd, stderr, mã thoát, `luc ext …` và `--trace-startup`. Không dùng `open -a`, vì nó bỏ stdout/stderr và mã thoát.
- **Lệch khỏi cả hai:** chưa có app một instance qua IPC. Mỗi `luc` vẫn là một tiến trình như trước. Đây là việc làm sau.
- **Mở từ Finder hoặc Dock** (không có đường dẫn, cwd là `/`): hiện hộp thoại chọn thư mục, giống "Open Folder" của VS Code; bấm Hủy thì thoát. Không mở `/`, vì quét cả ổ đĩa vừa chậm vừa vô ích.
- **`cargo run`** giữ như cũ: không có bundle, menu bar hiện "luc".
