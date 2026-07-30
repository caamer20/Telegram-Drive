# Feature Specification: Android Phase 1 Vertical Slice

**Feature Branch**: `agent/android-phase-0`

**Created**: 2026-07-30

**Status**: Ready

**Input**: Chứng minh luồng Android xuyên tầng từ authorization thật, khôi phục session, Saved Messages, download đến preview ảnh và phát video local.

## Ngôn ngữ

Toàn bộ specification viết bằng Tiếng Việt; tên giao thức, trạng thái và thư viện giữ nguyên tiếng Anh.

## User Scenarios & Testing

### User Story 1 - Đăng nhập và khôi phục session (Priority: P1)

Người dùng cấu hình API credential local, đi qua đúng authorization state Telegram yêu cầu, đăng nhập bằng phone/code/2FA khi cần và mở lại ứng dụng mà không nhập lại OTP khi session còn hợp lệ.

**Why this priority**: Mọi dữ liệu thật phụ thuộc authorization và session an toàn.

**Independent Test**: Có thể kiểm thử bằng fake state machine không cần credential; runtime thật được chứng minh bằng login thủ công, force-stop và reopen.

**Acceptance Scenarios**:

1. **Given** chưa có session, **When** TDLib phát state yêu cầu input, **Then** UI chỉ cho phép action hợp lệ và không log/lưu OTP hoặc password.
2. **Given** account có 2FA hoặc email/other-device confirmation, **When** state tương ứng xuất hiện, **Then** UI phản ánh đúng state hoặc thông báo an toàn nếu chưa hỗ trợ action cụ thể.
3. **Given** session hợp lệ, **When** force-stop và reopen, **Then** UI giữ trạng thái initializing cho đến khi Ready và không yêu cầu OTP lại.
4. **Given** người dùng logout/reset, **When** thao tác hoàn tất, **Then** client được đóng có chủ đích và account state trở về auth an toàn.

### User Story 2 - Xem media gần nhất trong Saved Messages (Priority: P2)

Người dùng đã đăng nhập mở Saved Messages và thấy một danh sách giới hạn các ảnh, video, animation hoặc document gần nhất với loading/empty/error rõ ràng.

**Why this priority**: Đây là proof rằng authorization, account identity và data mapping hoạt động xuyên tầng.

**Independent Test**: Fake source cung cấp cùng state shape và danh sách có item hợp lệ, item ngoài scope và duplicate để chứng minh lọc/deduplicate.

**Acceptance Scenarios**:

1. **Given** authorization Ready, **When** mở thư viện, **Then** hệ thống xác định đúng self/Saved Messages và chỉ tải một batch gần nhất.
2. **Given** message trùng file hoặc content ngoài scope, **When** mapping, **Then** item không trùng và content ngoài scope được bỏ qua an toàn.
3. **Given** Saved Messages rỗng hoặc request lỗi, **When** hoàn tất, **Then** UI hiển thị empty/error thay vì danh sách stale.

### User Story 3 - Download và preview ảnh (Priority: P3)

Người dùng chọn ảnh, quan sát tiến trình download, có thể cancel/retry và chỉ mở preview khi local file hợp lệ.

**Why this priority**: Chứng minh file pipeline và preview đầu tiên với rủi ro lifecycle thấp hơn video.

**Independent Test**: Fake source chạy toàn flow success/failure/cancel mà không cần Telegram account.

**Acceptance Scenarios**:

1. **Given** ảnh chưa local, **When** người dùng chọn, **Then** chỉ một download bắt đầu và progress được cập nhật.
2. **Given** file tải xong và tồn tại, **When** mở preview, **Then** ảnh hiển thị trên screen riêng có back navigation.
3. **Given** file lỗi/mất/cancel, **When** preview được yêu cầu, **Then** không mở path invalid và UI cho phép retry phù hợp.

### User Story 4 - Download và phát video local (Priority: P4)

Người dùng chọn video, chờ download hoàn tất rồi phát local với play/pause/seek/back; player dừng và release khi rời screen.

**Why this priority**: Đây là bước cuối của vertical slice và phụ thuộc download/file validation đã có.

**Independent Test**: Fake source dùng video fixture local để kiểm thử routing và player lifecycle; runtime thật dùng một video Saved Messages đã tải xong.

**Acceptance Scenarios**:

1. **Given** video chưa local, **When** chọn, **Then** app download hoàn tất trước khi tạo player và không streaming.
2. **Given** local file hợp lệ, **When** mở player, **Then** play/pause/seek/back hoạt động và không tạo player trùng.
3. **Given** rời player hoặc Activity recreation, **When** lifecycle thay đổi, **Then** playback không tiếp tục ngoài ý muốn và resource được release.

### Edge Cases

- State authorization đến ngoài sequence phổ biến, lặp lại hoặc chuyển trong lúc request trước đang pending.
- API credential thiếu/sai, flood wait, message lỗi có dữ liệu nhạy cảm, mạng mất giữa auth/download.
- Recomposition hoặc repeated click gửi trùng action/download.
- Session hết hạn/corrupt, local path stale, file bị xóa sau download, progress update đến sau cancel.
- Saved Messages không có content trong scope hoặc nhiều message tham chiếu cùng file.
- Back/recreation trong lúc download hoặc playback.

## Requirements

### Functional Requirements

- **FR-001**: Hệ thống MUST xử lý initialization, wait parameters, phone, code, password, email address/code, other-device confirmation, ready, logging out, closing, closed và error theo state thực tế nhận được.
- **FR-002**: UI MUST không cho action sai state, chống duplicate submit và hiển thị state chưa hỗ trợ một cách an toàn.
- **FR-003**: API ID/hash MUST được cung cấp bằng local configuration bị ignore; không có giá trị thật trong repository, log, screenshot hoặc fixture.
- **FR-004**: OTP/password MUST không được persist hoặc log; error/flood wait MUST được sanitize trước khi hiển thị.
- **FR-005**: Session hợp lệ MUST được khôi phục sau force-stop/reopen; UI MUST không nhảy về login trong initialization.
- **FR-006**: Logout/reset MUST đóng client có chủ đích và xóa/reset account state theo contract; backup policy Phase 0 MUST tiếp tục bảo vệ dữ liệu.
- **FR-007**: Khi Ready, hệ thống MUST xác định self chat/Saved Messages và tải tối đa một batch gần nhất, không full scan/global index.
- **FR-008**: Mapping MUST hỗ trợ image, video, animation và document cơ bản, bỏ content ngoài scope, giữ stable identity và deduplicate file.
- **FR-009**: UI MUST có loading, empty và sanitized error cho auth, library, download và preview.
- **FR-010**: Download MUST hỗ trợ start/progress/complete/failure/cancel, chống duplicate và không block main thread; không cần sống qua process death.
- **FR-011**: Preview MUST chỉ dùng local path đang tồn tại; stale path MUST quay lại trạng thái có thể download/retry.
- **FR-012**: Image MUST hiển thị trên dedicated/full-screen screen có loading/error/back và không giữ reference sau close.
- **FR-013**: Video MUST chỉ phát local sau complete, có play/pause/seek/error/back và release đúng lifecycle; không streaming/local HTTP server.
- **FR-014**: Fake source MUST mô phỏng cùng auth/library/download/preview state shape và không load TDLib native.
- **FR-015**: Unit tests MUST bao phủ state mapping/transition, invalid/duplicate action, redaction, session startup, mapping/filter/dedup, download/cancel, preview routing, image error, player lifecycle, fake flow, logout và backup regression.
- **FR-016**: Runtime validation MUST bao gồm fake vertical slice, real login/session/Saved Messages/image/video khi credential và OTP thủ công có sẵn, activity recreation, force-stop/reopen và network loss.

### Security Requirements

- Dữ liệu account/session/download tiếp tục bị loại khỏi cloud backup và device transfer.
- Không commit hoặc chụp API credential, phone, OTP, password, TDLib database, keystore hay media cá nhân.
- Local configuration thiếu phải tạo trạng thái hướng dẫn, không fallback sang credential của app khác.

### Key Entities

- **AuthorizationSession**: lifecycle auth hiện tại, action pending, sanitized error và trạng thái session.
- **LocalApiCredentialStatus**: configured/missing/invalid; không chứa secret trong domain/UI state.
- **SavedMediaItem**: message/file stable identity, kind, metadata tối thiểu, download state và optional verified local path.
- **DownloadOperation**: file identity, progress, terminal state và cancellation generation.
- **PreviewDestination**: image hoặc video local hợp lệ gắn với media identity.

## Out of Scope

Channel browser đầy đủ, multi-source paging, full history scan, global gallery, Room index, background transfer, streaming trước download, upload, audio/PDF player hoàn chỉnh, local HTTP server, release và CI/CD.

## Deliverables

Spec/plan/tasks/design artifacts; authorization/session/Saved Messages/download/image/video vertical slice; fake flow; tests; runtime evidence; progress/report và danh sách P2.

## Validation Matrix

| Luồng | Unit | Fake runtime | Real runtime |
|---|---:|---:|---:|
| Authorization states/guard/redaction | Bắt buộc | Bắt buộc | Bắt buộc khi credential/OTP có sẵn |
| Session force-stop/reopen | Startup contract | Mô phỏng | Bắt buộc |
| Saved Messages mapping | Bắt buộc | Bắt buộc | Bắt buộc |
| Download/cancel/dedup | Bắt buộc | Bắt buộc | Ảnh + video |
| Image preview | Boundary/error | Bắt buộc | Bắt buộc |
| Video local playback/lifecycle | Boundary | Bắt buộc | Bắt buộc |
| Backup/credential leakage | Bắt buộc | Diff/log/screenshot | Diff/log/screenshot |

## Success Criteria

- **SC-001**: Fake vertical slice hoàn tất 100% từ auth đến image/video preview mà không load native client.
- **SC-002**: Một account thử nghiệm hoàn tất login theo state được yêu cầu và reopen không cần OTP lại khi session hợp lệ.
- **SC-003**: Một batch Saved Messages giới hạn hiển thị không duplicate và không quét toàn lịch sử.
- **SC-004**: Một ảnh và một video tải hoàn tất; ảnh preview được, video local play/pause/seek được, back không crash/leak.
- **SC-005**: Repeated click không tạo hơn một auth request, download hoặc player cho cùng operation.
- **SC-006**: Toàn bộ unit test, lint, real/fake debug build và Phase 0 regression pass; không phát hiện credential trong diff/log/evidence.

## Assumptions

- Developer cung cấp API credential qua file local ignored; OTP/other-device confirmation luôn là thao tác thủ công.
- Runtime chính là emulator ARM64 API 36 hiện có; thiết bị thật chỉ được claim khi thực sự kết nối.
- Batch gần nhất mặc định giới hạn 50 message; không paging trong proof of concept.
- Download không tiếp tục qua process death; Media3 được dùng cho video local theo yêu cầu.
