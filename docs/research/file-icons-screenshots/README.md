# Icon file có màu và icon trên tab: ảnh trước và sau

Chụp ngày 2026-11-15, cửa sổ 1470×923. Cả bốn ảnh dùng cùng thư mục mẫu (`src/app.ts`, `src/app.test.ts`, `src/main.rs`, `Dockerfile`, `docker-compose.yml`, `.env`, `.env.example`, `.gitignore`, `package.json`, `tsconfig.json`, `README.md`, `LICENSE`, `Cargo.toml`, `notes.txt`, `logo.png`, `Makefile`), cùng 7 tab mở sẵn (khôi phục từ file trạng thái workspace), một HOME tạm và không có extension.

| | Tối | Sáng |
|---|---|---|
| Trước (icon theme Lucide, `target/release/luc` dựng từ main) | `before-dark.png` | `before-light.png` |
| Sau (Seti mặc định, nhánh `claude/file-icons`) | `after-dark.png` | `after-light.png` |

- Theme được chọn bằng `workbench.colorTheme` và `"window.autoDetectColorScheme": false`.
- Tab file có icon cùng cỡ 14 px với icon trong Explorer. Thư mục vẫn dùng icon folder Lucide.
- Trên theme sáng, các màu dưới 3:1 (vàng, xám) đã được làm đậm: xem `package.json`, `LICENSE`, `notes.txt`.
- Dải "Restricted Mode" có mặt vì thư mục mẫu chưa được tin cậy.
