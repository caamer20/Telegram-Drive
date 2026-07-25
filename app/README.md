# Tauri + React + Typescript

This template should help get you started developing with Tauri, React and Typescript in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Dependency xử lý video

OneDrive migration dùng `ffmpeg` và `ffprobe` để nhận diện, nén và chuẩn hóa video sang H.264 tối đa Full HD. Hai binary phải:

- nằm cạnh nhau trong resource directory của ứng dụng; hoặc
- có thể gọi trực tiếp từ `PATH`.

Kiểm tra môi trường:

```bash
ffmpeg -version
ffprobe -version
```

Tệp không phải video vẫn được upload nguyên trạng. Video H.264 không vượt 1920×1080 được bỏ qua chuyển mã; các video còn lại được tạo thành MP4 H.264/AAC mà không sửa tệp nguồn.
