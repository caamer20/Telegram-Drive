# Capability Spike Report: Persisted Telegram random_id Idempotency

Báo cáo này phân tích giới hạn của thư viện Grammers API cao cấp đối với việc tùy biến `random_id` và đề xuất giải pháp API thô (raw API calls) để bảo đảm gửi idempotent.

---

## 1. Revision Grammers & High-Level Limitation

*   **Grammers Revision**:
    Dự án hiện đang ghim phiên bản Grammers tại:
    ```toml
    grammers-client = { git = "https://github.com/Lonami/grammers", rev = "d07f96f" }
    ```
*   **Hạn chế của API cao cấp (High-Level API Limitation)**:
    Phương thức `Client::send_message(&self, chat: &Peer, message: InputMessage) -> Result<Message, InvocationError>` tự động sinh ngẫu nhiên số `random_id` 64-bit bên trong thân hàm mà không cho phép truyền từ bên ngoài qua struct `InputMessage`.
    ```rust
    // Định nghĩa nội bộ trong grammers (rev d07f96f)
    let random_id = rand::random::<i64>();
    ```
    Điều này khiến cho việc retry gửi media/tệp tin sau khi ứng dụng crash hoặc khởi động lại sẽ luôn sinh ra một `random_id` mới, làm mất hoàn toàn tính năng chống trùng lặp (de-duplication) trên máy chủ Telegram.

---

## 2. Raw API Path & Request Construction

Để tiêm được `telegram_random_id` được lưu trữ bền vững trong cơ sở dữ liệu SQLite, chúng ta bắt buộc phải sử dụng raw API `Client::invoke` với hàm `SendMedia` của Telegram MTProto:

*   **Hàm MTProto**: `messages::SendMedia`
*   **Struct API**: `grammers_tl_types::functions::messages::SendMedia`
*   **Input File API**: `grammers_tl_types::enums::InputMedia::UploadedDocument`

### Sơ đồ luồng gửi tệp tin raw:
1.  Upload binary lên Telegram thông qua:
    `let uploaded_file = client.upload_stream(&mut reader, size as usize, file_name).await?;`
2.  Xây dựng raw `InputMediaUploadedDocument`:
    ```rust
    let media = tl::enums::InputMedia::UploadedDocument(tl::types::InputMediaUploadedDocument {
        nosound_video: false,
        force_file: true,
        spoiler: false,
        file: uploaded_file.input_file, // Trích xuất InputFile
        thumb: None,
        mime_type: "application/octet-stream".to_string(),
        attributes: vec![],
        ttl_seconds: None,
    });
    ```
3.  Gọi invoke trực tiếp qua `Client`:
    ```rust
    let updates = client.invoke(&tl::functions::messages::SendMedia {
        silent: false,
        background: false,
        clear_draft: false,
        noforwards: false,
        update_stickersets_order: false,
        invert_media: false,
        peer: input_peer,
        reply_to: None,
        media,
        message: "".to_string(),
        random_id: persisted_random_id, // Tiêm persisted random_id từ SQLite
        reply_markup: None,
        entities: None,
        schedule_date: None,
        send_as: None,
    }).await?;
    ```

---

## 3. Kết quả Biên dịch & Test Coverage

*   **Compile Status**: **Thành công**.
    Spike đã được triển khai độc lập trong module `telegram_idempotency.rs` và biên dịch thành công 100% với Cargo.
*   **Test Coverage**:
    *   `test_deterministic_random_id`: Chứng minh cùng một `upload_attempt_id` luôn sinh ra cùng một `telegram_random_id` 64-bit mang tính deterministic bằng SHA-256 băm.
    *   `test_map_short_sent_message`: Xác nhận giải mã thành công response `UpdateShortSentMessage` lấy ra message ID.
    *   `test_map_short_unsupported`: Xác nhận các response update lạ tự động map về `reconciliation_required` để đảm bảo an toàn.

---

## 4. Rủi ro Dependency & Khuyến nghị Production

### Rủi ro Dependency (Dependency Risks):
*   API thô `tl::functions::messages::SendMedia` là các kiểu dữ liệu tự động sinh từ lược đồ layer Telegram. Nó không thuộc tầng API ổn định (Stability Guarantee) của Grammers Client và có thể thay đổi tham số khi nâng cấp layer MTProto.
*   Việc parse thủ công struct `Updates` yêu cầu phải bao phủ đầy đủ các variant phản hồi của Telegram (như `UpdateShortSentMessage`, `UpdatesCombined`, `Updates`). Nếu bỏ sót, item sẽ bị kẹt ở trạng thái `reconciliation_required`.

### Khuyến nghị Production:
*   **Quyết định**: **NO-GO** cho việc tự động thay thế toàn bộ production upload sang raw path ngay lập tức.
*   **Lý do**: Layer MTProto thô của Grammers đòi hỏi kiểm thử thực tế rộng rãi trên nhiều loại chat peer (user, channel, group) để đảm bảo không bị crash do thiếu entity cache hoặc mismatch peer format.
*   **Chiến lược trung hạn**:
    *   Spike đã chứng minh khả năng tiêm `random_id` thành công.
    *   Giữ nhánh upload production hiện tại (`upload_core` cao cấp) làm luồng chính.
    *   Triển khai raw API path này trong một module tách biệt hoặc ở chế độ Opt-in thử nghiệm trước khi triển khai đại trà ở Vòng 2B.
