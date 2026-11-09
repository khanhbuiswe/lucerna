# Lucerna — Landing page

- **Ngày:** 2026-11-09
- **Trạng thái:** Bản nháp chờ duyệt
- **Mockup đã duyệt:** [`2026-11-09-landing-page-mockup.html`](2026-11-09-landing-page-mockup.html) (bản xem trực tuyến: https://claude.ai/artifact/Xh8wbLS9o9Q2c67mk8yuVx)

## 1. Mục tiêu

Một trang giới thiệu Lucerna, chuyên nghiệp ngang tầm trang của Wu (wu.dstrml.org), nhưng có bản sắc riêng và chỉ nói đúng những gì Lucerna đang làm được.

Người xem là dev viết code chủ yếu qua agent trong terminal (Claude Code, Codex…). Trong 10 giây đầu, họ phải hiểu được ba điều:

1. Lucerna là trình xem code native, nhanh, viết bằng Rust, dành cho việc đọc code mà agent vừa sửa.
2. Lucerna không có AI bên trong, và giữ phím tắt cùng `settings.json` giống VS Code.
3. Cách cài đặt: bản release sắp ra mắt, hiện tại build từ source.

**Tiêu chí thành công:**

- Trang deploy được lên GitHub Pages từ thư mục `site/`.
- Hiển thị đúng ở độ rộng 1280 px và 375 px, không cuộn ngang.
- Mọi con số và mọi khẳng định trên trang đều kiểm chứng được trong repo, xem mục 5.
- Lighthouse (mobile) đạt ≥ 95 cho Performance, Accessibility, Best Practices và SEO.

## 2. Những gì đã quyết định

| Câu hỏi | Quyết định |
|---|---|
| Nơi đặt và cách host | HTML/CSS thuần trong `site/` của repo này, không có bước build, deploy bằng GitHub Pages |
| CTA chính | Khu download thiết kế sẵn (lệnh `curl`, nút theo nền tảng), gắn nhãn "Soon" cho tới khi có release; kèm lệnh build từ source |
| Ngôn ngữ | Chỉ tiếng Anh |
| Ảnh editor trong hero | Dựng bằng HTML/CSS theo màu One Dark, không dùng ảnh chụp |
| Số liệu hiệu năng | Chỉ dùng số đã đo trong `docs/perf-checklist.md`; số chưa đo ghi là mục tiêu; không so sánh với VS Code hay Zed |
| Giao diện | Chỉ có giao diện tối (Lucerna mặc định One Dark); màu nhấn là vàng hổ phách, lấy từ ý "ngọn đèn" |

**Ngoài phạm vi:** trang docs, blog, bản tiếng Việt, analytics, form đăng ký nhận tin, trang changelog, script `install.sh` thật.

## 3. Cấu trúc file

```
site/
  index.html          # toàn bộ nội dung, HTML có ngữ nghĩa
  styles.css          # token màu/chữ ở :root, rồi từng section
  main.js             # chỉ có nút Copy (~20 dòng), trang vẫn dùng được khi tắt JS
  favicon.svg         # logo ngọn đèn, giống logo trên thanh nav
  og.png              # ảnh 1200×630 để chia sẻ link (chụp từ phần hero)
  fonts/              # woff2 tự host, kèm file license OFL
    BricolageGrotesque-*.woff2
    InstrumentSans-*.woff2
    JetBrainsMono-*.woff2
    OFL-*.txt
.github/workflows/pages.yml
```

- **Tự host font, không gọi Google Fonts.** Trang không gửi request nào tới bên thứ ba, đúng với câu "no telemetry" trong FAQ. Chỉ subset Latin, mỗi font tối đa hai weight. Ba font đều dùng giấy phép OFL 1.1, được ghi thêm vào `THIRD_PARTY_NOTICES.md`.
- `<head>` gồm: `title`, `meta description`, Open Graph và Twitter card, `theme-color`, `link rel=canonical`, preload font hiển thị chính.
- Không dùng framework hay thư viện JS, cũng không có bước build.

## 4. Các section

Thứ tự và nội dung lấy theo mockup. Văn bản trong mockup là văn bản chính thức, chỉ đổi những chỗ ghi ở mục 6.

| # | Section | Nội dung chính |
|---|---|---|
| 1 | Nav (sticky) | Logo + "Lucerna"; các link Speed, Features, Roadmap, GitHub; nút "Download · Soon" dẫn tới `#download` |
| 2 | Hero | Eyebrow "Open source · Written in Rust · macOS"; H1 "A quiet place to *read* the code your agents write."; đoạn mô tả; lệnh `curl` + nút Copy + nhãn "First release coming soon" + link build từ source; khung editor mock và ô terminal của agent |
| 3 | Speed (`#speed`) | 4 số đo thật: 887 ms, 2.04 ms, 96 µs, 51 ms; thanh so với ngân sách cho hai số đầu; ghi chú cách đo |
| 4 | Features (`#features`) | 6 tính năng giai đoạn 1 bên phải; danh sách 6 phím tắt bên trái (sticky trên desktop) |
| 5 | Principles | No AI inside · Native · Open source |
| 6 | Roadmap (`#roadmap`) | 5 giai đoạn theo spec giai đoạn 1; giai đoạn 1 được làm nổi bật |
| 7 | Download (`#download`) | Lệnh build từ source + Copy; macOS Apple Silicon và Intel gắn "Soon"; Linux và Windows mờ, ghi "after Phase 1" |
| 8 | FAQ | 5 câu hỏi dùng `<details>`, câu đầu mở sẵn |
| 9 | Footer | "Latin for lamp", giấy phép, các link GitHub, Roadmap, Report a problem |

**Khung editor mock trong hero:**

- Dựng bằng HTML, có `role="img"` và `aria-label` mô tả nội dung.
- Code bên trong lấy từ `crates/lucerna-text/src/buffer.rs`: dòng 59–82, gồm hàm `reload()`.
- Dòng 65–77 tô xanh kiểu "vừa thay đổi". Khi load trang, các dòng này nháy nhẹ một lần, và không nháy nếu người dùng bật `prefers-reduced-motion`.
- Explorer đánh dấu `buffer.rs` và `reload.rs` là bị sửa từ bên ngoài. Tab `reload.rs` chữ nghiêng (preview tab). Status bar hiện "Reloaded from disk".
- Ô terminal của agent chỉ để trang trí (`aria-hidden`) và bị ẩn khi màn hình hẹp hơn 960 px.
- Nếu code của `buffer.rs` thay đổi, đoạn code trong mock **không** cần cập nhật theo. Nó chỉ là minh hoạ.

**Responsive:**

- Từ 960 px trở xuống: ẩn ô terminal, phần Features còn một cột, roadmap còn hai cột.
- Từ 640 px trở xuống: ẩn link trên nav, ẩn explorer và minimap của mock, mọi lưới còn một cột.
- Lệnh dài cuộn ngang bên trong khung của nó, không làm cả trang cuộn ngang.

## 5. Nguồn của mọi khẳng định

| Khẳng định trên trang | Nguồn trong repo |
|---|---|
| 887 ms quét 80.000 file; 2.04 ms Quick Open; 96 µs highlight; 51 ms reload | `docs/perf-checklist.md`, bảng Automated, đo ngày 2026-11-09 |
| Ngân sách 5 s và 16 ms; mục tiêu 200 ms, 150 MB, 120 fps | Spec giai đoạn 1, mục 6 |
| CI chặn khi benchmark chậm hơn 15% | `scripts/bench-gate.sh`, job `bench` trong `ci.yml` |
| Không có dependency GPL, kiểm bằng `cargo deny` | `deny.toml`, `ci.yml` |
| Search dùng engine của ripgrep | các crate `grep-searcher`, `grep-regex` trong `Cargo.toml` |
| Không telemetry; log nằm ở `~/Library/Logs/Lucerna` | không crate nào gọi mạng (không có reqwest/hyper/ureq); `crates/lucerna/src/logging.rs` |
| Chỉ macOS 13 trở lên; Linux và Windows làm sau giai đoạn 1 | Spec giai đoạn 1, dòng 92 |
| Tính năng, phím tắt, lộ trình 5 giai đoạn | Spec giai đoạn 1, mục 1.3, 2.1 và 5.4 |

Khi một trong các nguồn này thay đổi (ví dụ đo lại benchmark hoặc ra bản release đầu tiên), trang phải được cập nhật trong cùng PR đó. Dòng này sẽ được thêm vào `docs/perf-checklist.md`.

## 6. Những chỗ khác so với mockup

1. Mockup dùng Google Fonts; bản thật tự host font (mục 3).
2. Mockup bỏ qua `<!doctype>` và `<head>` vì nền tảng artifact tự thêm vào; bản thật phải có đầy đủ.
3. **Tên miền:** chưa có. Lệnh `curl` và thẻ canonical dùng một hằng số duy nhất, tạm đặt là `https://lucerna.dev`. Nếu khi deploy vẫn chưa có tên miền thì dùng địa chỉ mặc định của GitHub Pages (`https://lucerna-dev.github.io/lucerna/`). Mọi link nội bộ đều là đường dẫn tương đối, nên trang chạy được ở cả hai địa chỉ.
4. **Repo GitHub:** link dùng `https://github.com/lucerna-dev/lucerna`, lấy từ `Cargo.toml`. Repo hiện chưa có remote, nên link sẽ hỏng cho tới khi repo được đẩy lên.
5. Nút "Download" trên nav trỏ tới `#download`, không tải file nào.

## 7. Deploy

File `.github/workflows/pages.yml`:

- Chạy khi push lên `main` có thay đổi trong `site/**` hoặc chính file workflow này; chạy tay được bằng `workflow_dispatch`.
- Dùng `actions/configure-pages`, `actions/upload-pages-artifact` (đường dẫn `site`) và `actions/deploy-pages`.
- Quyền tối thiểu: `pages: write`, `id-token: write`, `contents: read`; `concurrency: pages`.
- Không ảnh hưởng tới `ci.yml` hiện có.

Bật GitHub Pages (Source: GitHub Actions) trong cài đặt repo là việc tác giả tự làm sau khi có remote.

## 8. Kiểm tra

Đây là trang tĩnh nên không viết unit test. Kiểm tra bằng các bước sau:

1. Mở `site/index.html` trực tiếp từ đĩa và qua `python3 -m http.server`, ở độ rộng 1280 px và 375 px: không cuộn ngang, font tải đúng, không có lỗi trong console.
2. Tắt JS: trang vẫn đọc được, lệnh vẫn chọn được để copy.
3. Bật `prefers-reduced-motion`: không có animation.
4. Dùng bàn phím: Tab đi hết các link, nút và `<details>` theo đúng thứ tự, focus luôn nhìn thấy được.
5. Lighthouse mobile ≥ 95 cho cả bốn mục.
6. Validator HTML của W3C không báo lỗi.
7. Tab Network chỉ có request tới chính origin của trang.
8. Đối chiếu từng dòng trong bảng ở mục 5 với nội dung trên trang.
