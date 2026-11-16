# Bundle id riêng cho bản dev — thiết kế ngắn

## Vì sao

Mở folder gần đây bằng App Exposé (Ctrl+↓) lúc được lúc không. Log cho
thấy lần hỏng không có yêu cầu mở nào tới app đang chạy. Lúc đó app đang
chạy là bản `target/release/Lucerna.app`. Với bản `/Applications/Lucerna.app`,
lần nào cũng mở đúng (người dùng thử khoảng 15 lần ngày 2026-11-16, cộng ba
lần `open -b`).

Nguyên nhân: mọi bản build đều mang bundle id `dev.lucerna.Lucerna`
(`bundle-app.sh` chép nguyên `Info.plist`). LaunchServices đang ghi nhận 6 bản
như vậy, gồm bản trong `target/`, `~/LucernaBench.noindex`, DMG đã tháo và
thư mục tạm của session khác. Khi macOS mở tài liệu theo bundle id, nó có thể
chọn một bản khác bản đang chạy, kể cả bản đã bị xoá, nên yêu cầu mở không
tới app.

## Mục tiêu

- Bản chạy từ cây build (không cài) có bundle id và tên riêng, nên Ctrl+↓,
  Dock và "Open With" của nó không lẫn với bản chính.
- Bản cài vào `/Applications`, DMG và bản phát hành trên GitHub giữ nguyên
  `dev.lucerna.Lucerna` / "Lucerna".

## Không làm

- Không tách dữ liệu: bản dev vẫn dùng chung `~/Library/Application Support/Lucerna`,
  log và socket `luc`, vì các đường dẫn này đặt tên cứng, không theo bundle
  id. Chạy cùng lúc hai bản thì `luc` chỉ nói chuyện với một bản, như hiện
  nay.
- Không làm icon riêng cho bản dev.
- Không sửa code mở folder và cách chọn cửa sổ (`window_choice`), vì đã đúng.

## Cách đã cân nhắc

1. **Bản dev có bundle id riêng (đã chọn).** Đây là cách Zed làm:
   `dev.zed.Zed`, `dev.zed.Zed-Preview`, `dev.zed.Zed-Dev`. VS Code cũng
   tách Insiders thành `com.microsoft.VSCodeInsiders`. Hạn chế: các bản dev
   vẫn trùng id với nhau.
2. Chỉ đổi cách làm việc: luôn `--install` rồi gỡ đăng ký các bản cũ bằng
   `lsregister -u`. Bỏ vì dễ tái phát, do session khác cứ build ra bản mới.
3. Cả 1 và thêm bước gỡ đăng ký các bản dev cũ. Để sau, nếu cách 1 chưa đủ.

## Thiết kế

- `crates/lucerna/resources/macos/Info.plist`: `CFBundleIdentifier` và
  `CFBundleName` (cùng `CFBundleDisplayName` nếu có) thành placeholder
  `@BUNDLE_ID@` và `@BUNDLE_NAME@`.
- `scripts/bundle-app.sh` có hai kênh, giống Zed (mặc định là dev, bản phát
  hành phải nói rõ):
  - mặc định: `dev.lucerna.Lucerna-Dev`, "Lucerna Dev";
  - `--stable`: `dev.lucerna.Lucerna`, "Lucerna". `--install` tự bật
    `--stable`.
  - Tên thư mục `.app` và tên file binary giữ nguyên `Lucerna.app` / `luc`,
    để các script và tài liệu đang trỏ tới chúng không phải đổi.
- `scripts/make-dmg.sh` và `.github/workflows/release.yml` truyền `--stable`.
- Giữ ba dòng log chẩn đoán đã thêm trong
  `crates/lucerna-workbench/src/app.rs`: yêu cầu mở và mở lại từ hệ điều
  hành, cùng các đường dẫn đọc được.

## Để người làm quyết định

- Cách kiểm tra plist sau khi build: `plutil`, `defaults read`.
- Cập nhật `docs/research/2026-11-09-app-bundle.md` để ghi lại hai kênh.

## Tham khảo

- Zed: kênh phát hành (stable, preview, nightly, dev) quyết định bundle id
  và tên app. Bản build mặc định là dev.
- VS Code: Stable và Insiders có bundle id riêng nên cài song song được.
