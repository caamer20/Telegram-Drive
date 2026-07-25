# Checklist Chất lượng Yêu cầu Media

**Purpose**: Đánh giá độ đầy đủ, rõ ràng và kiểm thử được của yêu cầu tối ưu video trước implementation
**Created**: 2026-07-24

## Requirement Completeness

- [x] CHK001 Các điều kiện phân biệt non-video, video passthrough và video cần chuyển mã đã được định nghĩa đầy đủ chưa? [Completeness, Spec §FR-002–FR-006]
- [x] CHK002 Yêu cầu đầu ra đã bao phủ codec, giới hạn kích thước, tỷ lệ hình, chống upscale và khả năng phát lại chưa? [Completeness, Spec §FR-003–FR-004]
- [x] CHK003 Ownership của tệp nguồn, tệp tạm và trách nhiệm cleanup đã được mô tả cho mọi kết quả chưa? [Completeness, Spec §FR-007–FR-009]
- [x] CHK004 Yêu cầu hủy, retry và phục hồi sau restart đã được định nghĩa cho bước xử lý dài hạn chưa? [Completeness, Spec §FR-009, §FR-012]

## Requirement Clarity

- [x] CHK005 “Full HD” đã được định nghĩa rõ là bounding box tối đa 1920×1080 thay vì ép đúng resolution chưa? [Clarity, Spec §Assumptions]
- [x] CHK006 “Chất lượng thấp hơn” đã được phân biệt rõ giữa resolution thấp và codec không tương thích chưa? [Clarity, Spec §US2/AC1–AC2]
- [x] CHK007 Điều kiện “đủ tối ưu để bỏ qua” có định nghĩa codec và resolution cụ thể, không dựa trên cảm tính chưa? [Clarity, Spec §FR-005–FR-006]
- [x] CHK008 Chính sách giữ tỷ lệ cho video dọc và rotation metadata đã rõ ràng chưa? [Clarity, Spec §US1/AC2, Edge Cases]

## Requirement Consistency

- [x] CHK009 Yêu cầu passthrough video thấp hơn Full HD có nhất quán với yêu cầu bắt buộc H.264 không? [Consistency, Spec §FR-005–FR-006]
- [x] CHK010 Quy tắc non-video không bị mâu thuẫn giữa nhận diện nội dung và giữ luồng upload hiện tại chưa? [Consistency, Spec §FR-002, §FR-011]
- [x] CHK011 Quy tắc xóa source OneDrive chỉ sau upload thành công có nhất quán với ownership/cleanup không? [Consistency, Spec §US1/AC3, §FR-007–FR-009]

## Acceptance Criteria Quality

- [x] CHK012 Các tiêu chí codec, bounding box, tỷ lệ và chống upscale có thể đo khách quan không? [Measurability, Spec §SC-001–SC-003]
- [x] CHK013 Tiêu chí cleanup có bao phủ thành công, lỗi và hủy với kết quả quan sát được không? [Measurability, Spec §SC-005]
- [x] CHK014 Tiêu chí lỗi có yêu cầu thông tin chẩn đoán cụ thể thay vì chỉ “thân thiện” hoặc “rõ ràng” không? [Measurability, Spec §SC-006]

## Scenario Coverage

- [x] CHK015 Primary flow cho video độ phân giải cao đã được định nghĩa độc lập và kiểm thử được chưa? [Coverage, Spec §US1]
- [x] CHK016 Alternate flow cho H.264 thấp, codec khác H.264 và non-video đã đủ chưa? [Coverage, Spec §US2]
- [x] CHK017 Exception và recovery flow cho probe lỗi, encode lỗi, cancel và restart đã đủ chưa? [Coverage, Spec §US3]

## Edge Case Coverage

- [x] CHK018 Các yêu cầu đã bao phủ rotation, video dọc, kích thước chẵn và không audio chưa? [Coverage, Spec §Edge Cases]
- [x] CHK019 Các yêu cầu đã bao phủ Unicode path, disk full, file giả video và output zero-byte chưa? [Coverage, Spec §Edge Cases, §FR-013]

## Dependencies & Assumptions

- [x] CHK020 Dependency công cụ media và hành vi khi dependency thiếu đã được ghi nhận rõ chưa? [Dependency, Spec §Assumptions, §US3]
- [x] CHK021 Boundary giữa OneDrive migration và các upload flow ngoài phạm vi đã được ghi rõ chưa? [Scope, Spec §Assumptions]
- [x] CHK022 Giới hạn lifecycle chỉ khi Tauri process chạy đã được ghi rõ và phù hợp constitution chưa? [Assumption, Spec §Assumptions]
