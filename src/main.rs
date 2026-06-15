mod config; // Imports config.rs module

use base64::{Engine as _, engine::general_purpose};
use futures_util::StreamExt;
use image::{ImageBuffer, Rgb};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
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
async fn generate_image(
    params: &HashMap<String, Value>,
) -> Result<ImageBuffer<Rgb<u8>, Vec<u8>>, Box<dyn std::error::Error>> {
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

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim().to_string();
            buffer.drain(..=line_end);

            if line.is_empty() || !line.starts_with("data: ") {
                continue;
            }

            let data_str = &line[6..];

            if data_str == "[DONE]" {
                break;
            }

            let msg: Value = match serde_json::from_str(data_str) {
                Ok(json) => json,
                Err(_) => {
                    eprintln!(
                        "Warning: Received a fragmented or truncated JSON event string from server."
                    );
                    continue;
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
                    return Err(
                        "Unsupported channel count. Only 3-channel (RGB) is supported.".into(),
                    );
                }

                let img_buffer = ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(
                    complete_msg.width,
                    complete_msg.height,
                    image_bytes,
                )
                .ok_or("Failed to construct image from raw buffer")?;

                return Ok(img_buffer);
            }
        }
    }

    Err("Stream ended without receiving complete image payload".into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Ensure the 'outputs' directory exists
    fs::create_dir_all("outputs")?;

    // Determine the next starting sequential file index using regex matching
    let index_pattern = Regex::new(r"^image_(\d+)\.png$")?;
    let mut existing_indices = Vec::new();

    if let Ok(entries) = fs::read_dir("outputs") {
        for entry in entries.flatten() {
            if let Some(filename_str) = entry.file_name().to_str() {
                if let Some(caps) = index_pattern.captures(filename_str) {
                    if let Some(index_match) = caps.get(1) {
                        if let Ok(idx) = index_match.as_str().parse::<u32>() {
                            existing_indices.push(idx);
                        }
                    }
                }
            }
        }
    }

    // Next base index is determined safely without overwriting previous work
    let next_index = existing_indices.into_iter().max().unwrap_or(0) + 1;

    // Pull prompts from the isolated config module
    let available_prompts = config::get_prompts();
    let mut params_list = Vec::new();

    for prompt_text in available_prompts {
        let mut params = HashMap::new();
        params.insert("prompt".to_string(), json!(prompt_text));
        params.insert(
            "negative_prompt".to_string(),
            json!(config::NEGATIVE_PROMPT),
        );
        params.insert("size".to_string(), json!(512));
        params.insert("steps".to_string(), json!(50));
        params.insert("cfg".to_string(), json!(7.0));
        params.insert("use_opencl".to_string(), json!(true));
        params_list.push(params);
    }

    let total_images = params_list.len();
    for (i, params) in params_list.iter().enumerate() {
        println!("\nGenerating image {}/{}", i + 1, total_images);

        match generate_image(params).await {
            Ok(image) => {
                // Compute output name sequentially to mirror the Python runtime logic precisely
                let output_path = format!("outputs/image_{}.png", next_index + i as u32);
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
