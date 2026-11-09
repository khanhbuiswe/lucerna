# Lucerna — Logo

- **Ngày:** 2026-11-09
- **Trạng thái:** Bản nháp chờ duyệt
- **Hướng đã chọn:** J2 "Ánh sáng trên dòng chữ, tối giản", chọn từ 15 hướng và 8 biến thể vẽ thử trong buổi brainstorm

## 1. Mục tiêu

Thay logo tạm (giọt lửa trên bát đèn) bằng một logo có bản sắc riêng, dùng được từ icon app 1024 px xuống tới favicon 16 px.

Ý nghĩa: một vầng sáng nhô lên từ một dòng chữ, như ánh đèn rọi lên trang code. Hình vẫn giữ tinh thần tên "Lucerna" (Latin là đèn) và câu "A quiet place to read", nhưng không vẽ cái đèn ra.

**Phạm vi:** chỉ làm logo. Palette Cobalt, font và giọng văn giữ nguyên như spec landing page.

**Tiêu chí thành công:**

- Ở 16 px vẫn nhận ra là vầng sáng nằm trên một đường kẻ, không bị nhòe thành một vệt.
- Trên site, favicon, logo trên nav và `og.png` đều dùng logo mới. Không còn chỗ nào dùng logo đèn cũ.
- Có đủ file gốc trong `assets/brand/` để dùng cho README, icon app và bản phát hành sau này.
- `python3 scripts/check-site.py` vẫn chạy qua.

**Ngoài phạm vi:** tạo `.icns` và đóng gói `.app` (repo chưa có bước bundle), đổi palette, đổi font, làm brand guideline cho các kênh khác.

## 2. Hình dựng

Lưới `viewBox="0 0 64 64"`. Tâm vầng sáng ở (32, 42).

| Phần | Hình | Màu |
|---|---|---|
| Lớp sáng ngoài | Nửa hình tròn phía trên, r = 22 | accent `#4f8cff`, độ đậm 0.18 |
| Lớp sáng giữa | Nửa hình tròn phía trên, r = 14 | accent, độ đậm 0.45 |
| Lõi | Nửa hình tròn phía trên, r = 7 | accent, độ đậm 1 |
| Đường kẻ | `M4 44H60`, nét dày 5, đầu bo tròn | ink `#e6eaf2` |

Bản gốc (dùng từ 24 px trở lên):

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
  <path d="M10 42a22 22 0 0 1 44 0Z" fill="#4f8cff" opacity=".18"/>
  <path d="M18 42a14 14 0 0 1 28 0Z" fill="#4f8cff" opacity=".45"/>
  <path d="M25 42a7 7 0 0 1 14 0Z" fill="#4f8cff"/>
  <path d="M4 44H60" stroke="#e6eaf2" stroke-width="5" stroke-linecap="round"/>
</svg>
```

Các nửa hình tròn đều lấy đáy tại y = 42. Đường kẻ dày 5 phủ từ y = 41.5 đến 46.5, nên che kín mép đáy của vầng sáng và không để lộ khe hở nào.

## 3. Các biến thể

| Biến thể | Khác bản gốc | Dùng ở đâu |
|---|---|---|
| **Gốc** (`mark.svg`) | như mục 2 | từ 24 px trở lên: README, og, icon app |
| **Cỡ nhỏ** (`mark-small.svg`) | chỉ 2 lớp: lớp ngoài r = 22 độ đậm 0.3, lõi r = 10 độ đậm 1; đường kẻ dày 6 | dưới 24 px: favicon, logo trên nav |
| **Đơn sắc** (`mark-mono.svg`) | một màu, không trong suốt: lõi đặc r = 8, vòng cung r = 15 nét 3.5 không tô, đường kẻ; tất cả dùng `currentColor` | icon thanh menu, khắc, những chỗ không hiển thị được độ trong |
| **Nền sáng** | đường kẻ và chữ đổi sang `#080b12`; vầng sáng giữ nguyên | README trên GitHub ở chế độ sáng, bản in |

## 4. Logo ghép chữ

- Chữ "Lucerna" dùng Bricolage Grotesque, độ đậm 650, letter-spacing −0.02em. Đây là font tiêu đề đang dùng trên site.
- Logo đặt bên trái chữ. Logo dùng viewBox cắt sát `1 18 62 29` (chừa đủ chỗ cho đầu bo tròn của đường kẻ, chạy từ x = 1.5 đến 62.5), tức bỏ phần lưới trống phía trên và dưới.
- **Quy tắc chính:** mép dưới đường kẻ trong logo nằm trên đường chân chữ, để vầng sáng như mọc lên từ dòng chữ.
- Chiều cao logo (theo viewBox đã cắt) bằng khoảng 1.25 lần chiều cao chữ hoa. Khoảng cách giữa logo và chữ bằng 0.3 lần chiều cao logo.
- Trong file gốc, chữ được chuyển thành path, nên không phụ thuộc font cài trên máy. Path lấy từ `site/fonts/BricolageGrotesque-latin-wght.woff2` ở độ đậm 650, dùng fonttools. Font có giấy phép OFL, cho phép làm việc này.

## 5. Icon app macOS

- Canvas 1024×1024, ô bo góc rx ≈ 230 (tỷ lệ 22.5% như các icon macOS), nền `#0f1420`, viền 1.5% màu `#1b2232`.
- Đặt bản logo gốc vào giữa, đúng tỷ lệ trong mockup đã duyệt: toàn bộ logo (đường kẻ tính cả đầu bo tròn) rộng khoảng 60% canvas, tức scale 10 đơn vị lưới. Phần hình (x 1.5–62.5, y 20–46.5) được canh vào giữa ô: `translate(192 179.5) scale(10)`.
- Chỉ làm file gốc `app-icon.svg`. Việc xuất `.icns` để sang lúc đóng gói bản phát hành.

## 6. File thay đổi

```
assets/brand/                 # mới
  mark.svg                    # bản gốc
  mark-small.svg              # dưới 24 px
  mark-mono.svg               # currentColor
  lockup-dark.svg             # logo ghép chữ trên nền tối (chữ #e6eaf2)
  lockup-light.svg            # logo ghép chữ trên nền sáng (chữ #080b12)
  app-icon.svg                # 1024×1024
  README.md                   # bảng ngắn: file nào dùng ở đâu, màu, khoảng trống tối thiểu

site/favicon.svg              # thay bằng mark-small.svg (màu viết cứng); đường kẻ #080b12, đổi sang #e6eaf2
                              # qua @media (prefers-color-scheme: dark) bên trong SVG, để không mất trên tab sáng
site/index.html               # logo SVG trong .brand đổi sang bản cỡ nhỏ, dùng var(--accent) / var(--ink)
site/styles.css               # chỉnh .brand svg theo tỷ lệ mới và canh chân chữ
site/og.png                   # chụp lại phần hero sau khi đổi logo, vẫn 1200×630
docs/superpowers/specs/2026-11-09-landing-page-design.md   # sửa dòng "logo ngọn đèn" ở mục 3
```

Khoảng trống tối thiểu quanh logo bằng bán kính lõi (7 đơn vị lưới).

## 7. Kiểm tra

- `python3 scripts/check-site.py` và `python3 scripts/test_check_site.py` vẫn chạy qua.
- Mở site ở màn hình 1x và 2x, kiểm bằng mắt rằng đường kẻ của logo nằm trên chân chữ "Lucerna" trên nav.
- Xem favicon trên tab trình duyệt ở 16 px, cả tab sáng lẫn tab tối của trình duyệt. Cần nhìn ra vầng sáng nằm trên đường kẻ.
- Mở từng file trong `assets/brand/` trên nền sáng và nền tối. Riêng `mark-mono.svg` thì kiểm với một màu `color` bất kỳ.
- Tìm trong `site/` không còn path của logo cũ (`M12 2.5c2.6 3`).
