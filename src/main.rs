use base64::{engine::general_purpose, Engine as _};
use futures_util::StreamExt; // Brings .next() into scope for HTTP streaming
use image::{ImageBuffer, Rgb};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;

/// Struct representing the expected JSON structure of a 'complete' message
#[derive(Deserialize, Debug)]
struct CompleteMessage {
    generation_time_ms: u64,
    image: String,
    width: u32,
    height: u32,
    channels: u32,
}

/// Generate image using API (Fully Asynchronous & Stream-Safe)
async fn generate_image(params: &HashMap<String, Value>) -> Result<ImageBuffer<Rgb<u8>, Vec<u8>>, Box<dyn std::error::Error>> {
    let server_url = params
        .get("server_url")
        .and_then(|v| v.as_str())
        .unwrap_or("http://localhost:8081");

    let mut data = params.clone();
    data.remove("server_url");

    let prompt = params.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    println!("Generating: {}", prompt);

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/generate", server_url))
        .json(&data)
        .header("Accept", "text/event-stream")
        .send()
        .await?; 

    // Convert the response body into an active asynchronous byte stream
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Process any complete lines we have accumulated
        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim().to_string();
            buffer.drain(..=line_end); // Safely advance buffer slice

            if line.is_empty() || !line.starts_with("data: ") {
                continue;
            }

            let data_str = &line[6..];

            if data_str == "[DONE]" {
                break;
            }

            // DEFENSIVE PARSING: Catch malformed/cut-off JSON packets if the server crashes
            let msg: Value = match serde_json::from_str(data_str) {
                Ok(json) => json,
                Err(_) => {
                    eprintln!("Warning: Received a fragmented or truncated JSON event string from server.");
                    continue; // Skip processing this line and keep listening to the remaining bytes
                }
            };

            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

            if msg_type == "progress" {
                let step = msg.get("step").and_then(|v| v.as_u64()).unwrap_or(0);
                let total_steps = msg.get("total_steps").and_then(|v| v.as_u64()).unwrap_or(0);
                println!("Progress: {}/{}", step, total_steps);
            } else if msg_type == "complete" {
                let complete_msg: CompleteMessage = serde_json::from_value(msg)?;
                println!("Complete: {}ms", complete_msg.generation_time_ms);

                let image_bytes = general_purpose::STANDARD.decode(&complete_msg.image)?;

                if complete_msg.channels != 3 {
                    return Err("Unsupported channel count. Only 3-channel (RGB) is supported.".into());
                }

                let img_buffer = ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(
                    complete_msg.width,
                    complete_msg.height,
                    image_bytes,
                ).ok_or("Failed to construct image from raw buffer")?;

                return Ok(img_buffer);
            }
        }
    }

    Err("Stream ended without receiving complete image payload".into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let negative_prompt = "bad anatomy, bad hands, missing fingers, extra fingers, bad arms, missing legs, missing arms, poorly drawn face, bad face, fused face, cloned face, three crus, fused feet, fused thigh, extra crus, ugly fingers, horn, realistic photo, huge eyes, worst face, 2girl, long fingers, disconnected limbs,";

    let mut params_1 = HashMap::new();
    params_1.insert("prompt".to_string(), json!("cat playing with ball"));
    params_1.insert("negative_prompt".to_string(), json!(negative_prompt));
    params_1.insert("size".to_string(), json!(512));
    params_1.insert("steps".to_string(), json!(20));
    params_1.insert("cfg".to_string(), json!(7.0));
    params_1.insert("use_opencl".to_string(), json!(true));

    let mut params_2 = HashMap::new();
    params_2.insert("prompt".to_string(), json!("beautiful landscape, mountain"));
    params_2.insert("negative_prompt".to_string(), json!(negative_prompt));
    params_2.insert("size".to_string(), json!(512));
    params_2.insert("steps".to_string(), json!(20));
    params_2.insert("cfg".to_string(), json!(8.0));
    params_2.insert("use_opencl".to_string(), json!(true));

    let params_list = vec![params_1, params_2];

    fs::create_dir_all("outputs")?;

    let total_images = params_list.len();
    for (i, params) in params_list.iter().enumerate() {
        println!("\nGenerating image {}/{}", i + 1, total_images);

        match generate_image(params).await {
            Ok(image) => {
                let output_path = format!("outputs/image_{}.png", i + 1);
                image.save(&output_path)?;
                println!("Saved: {}", output_path);
            }
            Err(e) => {
                eprintln!("Error generating image {}: {}", i + 1, e);
            }
        }

        println!("{}", "-".repeat(50));
    }

    Ok(())
}