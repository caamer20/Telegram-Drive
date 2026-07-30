# Báo cáo Android Phase 1

## Trạng thái

Implementation và fake vertical slice hoàn tất. Real vertical slice đang bị block ở credential/OTP thủ công; vì vậy Phase 1 **chưa được tuyên bố hoàn thành**.

## Quyết định

- API ID/hash lấy từ file local ignored; repository chỉ giữ template trống.
- Auth UI phản ánh authorization state thực nhận và chỉ phát action hợp lệ; credential, OTP và password không được persist/log.
- Session TDLib nằm trong app-private `files/tdlib`, tiếp tục được bảo vệ bởi chính sách backup P0.
- Saved Messages chỉ lấy một batch tối đa 50, filter content trong scope và deduplicate theo file ID.
- Download là foreground operation, không sống qua process death; preview cần local file hợp lệ.
- Video chỉ phát file đã download bằng Media3 1.10.1; không streaming và không local HTTP server.

## Bằng chứng đã quan sát

- Unit tests bao phủ lifecycle/security P0 và auth/security/library/download/preview/player P1.
- Real và fake debug variants build; lint real pass.
- Fake runtime emulator arm64 API 36 hoàn tất auth giả lập, library, image và video flow.
- Image preview crash do recycled bitmap được phát hiện từ logcat, sửa và re-run thành công.
- Media3 controls play/pause/seek hiển thị từ local MP4 fixture; back không làm chết process.
- Clean real và fake matrix đều pass `testDebugUnitTest lintDebug assembleDebug`; tổng 20 unit tests pass.
- Real startup final load JNI, tạo client 1, báo TDLib 1.8.66/commit pinned và nhận `WaitTdlibParameters`.
- Gateway hạ TDLib log verbosity về 0 trước auth request để credential không đi vào logcat.

## Chưa thể xác minh

- Telegram login thật, OTP/2FA thật, force-stop session restore, Saved Messages/download/preview thật và network loss.
- Thiết bị Android vật lý và x86_64 runtime.

## Phạm vi P2

Paging/multi-source, global gallery, Room index, background transfer, streaming, upload, audio/PDF player đầy đủ và production release.

## Blocker tối thiểu

Developer cần tạo `android-app/telegram-api.properties` từ template bằng API ID/hash thử nghiệm và tự nhập phone/OTP/2FA trên runtime. Không gửi các giá trị này vào chat, git, script hoặc evidence.
