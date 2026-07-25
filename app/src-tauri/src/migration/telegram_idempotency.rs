use grammers_tl_types as tl;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Trait trừu tượng hóa việc invoke SendMedia của Telegram giúp mock dễ dàng trong test
pub trait TelegramInvoker: Send + Sync {
    fn invoke_send_media(
        &self,
        request: tl::functions::messages::SendMedia,
    ) -> impl std::future::Future<Output = Result<tl::enums::Updates, String>> + Send;
}

/// Helper sinh random_id 64-bit mang tính deterministic từ upload_attempt_id để chống trùng lặp
pub fn get_deterministic_random_id(upload_attempt_id: &str) -> i64 {
    let mut hasher = Sha256::new();
    hasher.update(upload_attempt_id.as_bytes());
    let result = hasher.finalize();

    // Lấy 8 bytes đầu tiên tạo i64
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&result[0..8]);
    i64::from_be_bytes(bytes)
}

/// Ánh xạ Updates response của Telegram sang message ID hoặc reconciliation_required bằng cách so sánh random_id
pub fn map_updates_response_v2(
    updates: &tl::enums::Updates,
    persisted_random_id: i64,
) -> Result<i64, String> {
    match updates {
        tl::enums::Updates::UpdateShortSentMessage(u) => Ok(u.id as i64),
        tl::enums::Updates::UpdateShort(u) => match &u.update {
            tl::enums::Update::MessageId(msg_id) => {
                if msg_id.random_id == persisted_random_id {
                    Ok(msg_id.id as i64)
                } else {
                    Err("reconciliation_required".to_string())
                }
            }
            _ => Err("reconciliation_required".to_string()),
        },
        tl::enums::Updates::Updates(u) => {
            for update in &u.updates {
                match update {
                    tl::enums::Update::MessageId(msg_id) => {
                        if msg_id.random_id == persisted_random_id {
                            return Ok(msg_id.id as i64);
                        }
                    }
                    tl::enums::Update::NewMessage(_new_msg) => {}
                    _ => {}
                }
            }
            Err("reconciliation_required".to_string())
        }
        _ => Err("reconciliation_required".to_string()),
    }
}

/// Cấu trúc mock invoker hỗ trợ unit test
pub struct MockTelegramInvoker {
    pub responses: std::sync::Mutex<HashMap<i64, Result<tl::enums::Updates, String>>>,
}

impl TelegramInvoker for MockTelegramInvoker {
    async fn invoke_send_media(
        &self,
        request: tl::functions::messages::SendMedia,
    ) -> Result<tl::enums::Updates, String> {
        let lock = self.responses.lock().unwrap();
        if let Some(res) = lock.get(&request.random_id) {
            res.clone()
        } else {
            Err("mock_not_found".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_random_id() {
        let attempt1 = "job_1_item_42_attempt_1";
        let attempt2 = "job_1_item_42_attempt_1";
        let attempt3 = "job_1_item_42_attempt_2";

        let id1 = get_deterministic_random_id(attempt1);
        let id2 = get_deterministic_random_id(attempt2);
        let id3 = get_deterministic_random_id(attempt3);

        // Cùng input -> cùng output
        assert_eq!(id1, id2);
        // Khác input -> khác output
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_map_short_sent_message() {
        let resp = tl::enums::Updates::UpdateShortSentMessage(tl::types::UpdateShortSentMessage {
            out: true,
            id: 12345,
            date: 100,
            media: Some(tl::enums::MessageMedia::Empty),
            entities: Some(vec![]),
            ttl_period: None,
            pts: 1,
            pts_count: 1,
        });

        assert_eq!(map_updates_response_v2(&resp, 999), Ok(12345));
    }

    #[test]
    fn test_map_short_unsupported() {
        let resp = tl::enums::Updates::UpdateShort(tl::types::UpdateShort {
            update: tl::enums::Update::Channel(tl::types::UpdateChannel { channel_id: 123 }),
            date: 100,
        });

        assert_eq!(
            map_updates_response_v2(&resp, 999),
            Err("reconciliation_required".to_string())
        );
    }

    #[test]
    fn test_map_update_message_id_scenarios() {
        let target_random_id = 123456789_i64;

        // 1. Matching random ID -> confirmed message ID
        let resp_match = tl::enums::Updates::UpdateShort(tl::types::UpdateShort {
            update: tl::enums::Update::MessageId(tl::types::UpdateMessageId {
                id: 555,
                random_id: target_random_id,
            }),
            date: 100,
        });
        assert_eq!(
            map_updates_response_v2(&resp_match, target_random_id),
            Ok(555)
        );

        // 2. Different random ID -> reconciliation_required
        let resp_diff = tl::enums::Updates::UpdateShort(tl::types::UpdateShort {
            update: tl::enums::Update::MessageId(tl::types::UpdateMessageId {
                id: 555,
                random_id: 999999999_i64, // Khác target
            }),
            date: 100,
        });
        assert_eq!(
            map_updates_response_v2(&resp_diff, target_random_id),
            Err("reconciliation_required".to_string())
        );

        // 3. Nhiều UpdateMessageId, chỉ một ID khớp -> chọn đúng ID
        let resp_multiple = tl::enums::Updates::Updates(tl::types::Updates {
            updates: vec![
                tl::enums::Update::MessageId(tl::types::UpdateMessageId {
                    id: 111,
                    random_id: 77777777_i64,
                }),
                tl::enums::Update::MessageId(tl::types::UpdateMessageId {
                    id: 222,
                    random_id: target_random_id, // Khớp ở đây
                }),
            ],
            users: vec![],
            chats: vec![],
            date: 100,
            seq: 0,
        });
        assert_eq!(
            map_updates_response_v2(&resp_multiple, target_random_id),
            Ok(222)
        );

        // 4. Không có ID khớp -> reconciliation_required
        let resp_none_match = tl::enums::Updates::Updates(tl::types::Updates {
            updates: vec![tl::enums::Update::MessageId(tl::types::UpdateMessageId {
                id: 111,
                random_id: 77777777_i64,
            })],
            users: vec![],
            chats: vec![],
            date: 100,
            seq: 0,
        });
        assert_eq!(
            map_updates_response_v2(&resp_none_match, target_random_id),
            Err("reconciliation_required".to_string())
        );
    }
}
