use std::io::Write;
use std::path::Path;

use grammers_client::types::{Media, Peer};
use grammers_client::{Client, InputMessage};
use grammers_tl_types as tl;

use crate::commands::utils::{map_error, resolve_peer};

pub async fn create_ciphertext_bucket(client: &Client, vault_id: &str) -> Result<i64, String> {
    let result = client
        .invoke(&tl::functions::channels::CreateChannel {
            broadcast: true,
            megagroup: false,
            title: "TelegramVault".to_string(),
            about: format!(
                "Telegram Drive encrypted vault bucket\n[telegram-drive-vault-v1:{}]",
                vault_id
            ),
            geo_point: None,
            address: None,
            for_import: false,
            forum: false,
            ttl_period: None,
        })
        .await
        .map_err(map_error)?;

    let (chat_id, access_hash) = match result {
        tl::enums::Updates::Updates(u) => {
            let chat = u
                .chats
                .first()
                .ok_or("No chat in channel creation response")?;
            match chat {
                tl::enums::Chat::Channel(c) => (c.id, c.access_hash.unwrap_or(0)),
                _ => return Err("Created bucket is not a Telegram channel".to_string()),
            }
        }
        _ => return Err("Unexpected channel creation response".to_string()),
    };

    let _ = client
        .invoke(&tl::functions::messages::SetHistoryTtl {
            peer: tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id: chat_id,
                access_hash,
            }),
            period: 0,
        })
        .await;

    Ok(chat_id)
}

pub async fn upload_object(
    client: &Client,
    bucket_id: i64,
    path: &Path,
    caption: String,
) -> Result<i32, String> {
    let peer = resolve_peer(client, Some(bucket_id)).await?;
    let path_str = path.to_string_lossy().to_string();
    let uploaded = client.upload_file(&path_str).await.map_err(map_error)?;
    let message = client
        .send_message(&peer, InputMessage::new().text(caption).file(uploaded))
        .await
        .map_err(map_error)?;
    Ok(message.id())
}

pub async fn download_object(
    client: &Client,
    bucket_id: i64,
    message_id: i32,
    output_path: &Path,
) -> Result<u64, String> {
    let peer = resolve_peer(client, Some(bucket_id)).await?;
    let messages = client
        .get_messages_by_id(&peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;
    let msg = messages
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| "Ciphertext object not found".to_string())?;
    let media = msg
        .media()
        .ok_or_else(|| "Ciphertext object has no media".to_string())?;

    let mut file = std::fs::File::create(output_path)
        .map_err(|e| format!("Failed to create ciphertext temp file: {}", e))?;
    let mut downloaded = 0u64;
    let mut download_iter = client.iter_download(&media);
    while let Some(chunk) = download_iter.next().await.transpose() {
        let bytes = chunk.map_err(|e| format!("Download chunk error: {}", e))?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        downloaded += bytes.len() as u64;
    }
    Ok(downloaded)
}

pub async fn delete_object(client: &Client, bucket_id: i64, message_id: i32) -> Result<(), String> {
    let peer = resolve_peer(client, Some(bucket_id)).await?;
    client
        .delete_messages(&peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[allow(dead_code)]
pub fn media_size(media: &Media) -> u64 {
    match media {
        Media::Document(d) => d.size() as u64,
        Media::Photo(_) => 1024 * 1024,
        _ => 0,
    }
}

#[allow(dead_code)]
pub fn peer_id(peer: &Peer) -> Option<i64> {
    match peer {
        Peer::Channel(channel) => Some(channel.raw.id),
        Peer::User(user) => Some(user.raw.id()),
        _ => None,
    }
}
