use actix_web::{get, web, App, HttpServer, HttpResponse, Responder};
use actix_cors::Cors;
use crate::commands::TelegramState;
use crate::commands::utils::resolve_peer;
use grammers_client::types::Media;

use std::sync::Arc;

#[get("/stream/{folder_id}/{message_id}")]
async fn stream_media(
    path: web::Path<(String, i32)>,
    data: web::Data<Arc<TelegramState>>,
) -> impl Responder {
    let (folder_id_str, message_id) = path.into_inner();
    
    // Parse folder ID
    let folder_id = if folder_id_str == "me" || folder_id_str == "home" || folder_id_str == "null" {
        None
    } else {
        match folder_id_str.parse::<i64>() {
            Ok(id) => Some(id),
            Err(_) => return HttpResponse::BadRequest().body("Invalid folder ID"),
        }
    };

    let client_opt = {
        data.client.lock().await.clone()
    };

    if let Some(client) = client_opt {
        match resolve_peer(&client, folder_id).await {
            Ok(peer) => {
                // Try to fetch message efficiently
                 match client.get_messages_by_id(peer, &[message_id]).await {
                    Ok(messages) => {
                        if let Some(Some(msg)) = messages.first() {
                            if let Some(media) = msg.media() {
                                let size = match &media {
                                    Media::Document(d) => d.size(),
                                    Media::Photo(_) => 0, 
                                    _ => 0,
                                };
                                
                                let mime = mime_type_from_media(&media);
                                
                                // Create chunk-streaming response
                                let mut download_iter = client.iter_download(&media);
                                let stream = async_stream::stream! {
                                    while let Some(chunk) = download_iter.next().await.transpose() {
                                        match chunk {
                                            Ok(bytes) => yield Ok::<_, actix_web::Error>(web::Bytes::from(bytes)),
                                            Err(e) => {
                                                log::error!("Stream error: {}", e);
                                                break;
                                            }
                                        }
                                    }
                                };
                                
                                return HttpResponse::Ok()
                                    .insert_header(("Content-Type", mime)) 
                                    .insert_header(("Content-Length", size.to_string()))
                                    .streaming(stream);
                            }
                        }
                        HttpResponse::NotFound().body("Message or media not found")
                    },
                    Err(e) => HttpResponse::InternalServerError().body(format!("Failed to fetch message: {}", e)),
                 }
            },
            Err(e) => HttpResponse::BadRequest().body(format!("Peer resolution failed: {}", e)),
        }
    } else {
        HttpResponse::ServiceUnavailable().body("Telegram client not connected")
    }
}

fn mime_type_from_media(media: &Media) -> String {
    match media {
        Media::Document(d) => d.mime_type().unwrap_or("application/octet-stream").to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

pub async fn start_server(state: Arc<TelegramState>, port: u16) -> std::io::Result<()> {
    let state_data = web::Data::new(state);
    
    log::info!("Starting Streaming Server on port {}", port);
    
    HttpServer::new(move || {
        App::new()
            .wrap(Cors::permissive())
            .app_data(state_data.clone())
            .service(stream_media)
    })
    .bind(("127.0.0.1", port))?
    .run()
    .await
}
