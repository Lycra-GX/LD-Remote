mod config;

use base64::{engine::general_purpose, Engine as _};
use futures_util::StreamExt;
use image::{ImageBuffer, Rgb};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Write};

/// Holds the decoded generate response
struct GenerateResult {
    image_bytes: Vec<u8>, // raw RGB pixel bytes
    width: u32,
    height: u32,
    channels: u32,
    format: String, // "raw", "png", "jpeg", etc.
}

/// Generate image using API (Fully Asynchronous & Stream-Safe)
async fn generate_image(
    params: &HashMap<String, Value>,
) -> Result<GenerateResult, Box<dyn std::error::Error>> {
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
    let mut final_json: Option<Value> = None;
    let mut buffer = String::new();
    let mut is_complete_event = false;

    while let Some(item) = stream.next().await {
        let chunk = item?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_index) = buffer.find('\n') {
            let line = buffer[..newline_index].to_string();
            buffer.drain(..=newline_index);

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if trimmed.starts_with("event:") {
                let event_type = trimmed.trim_start_matches("event:").trim();
                if event_type == "complete" {
                    is_complete_event = true;
                }
            } else if trimmed.starts_with("data:") && is_complete_event {
                let data_content = trimmed.trim_start_matches("data:").trim();
                if let Ok(parsed) = serde_json::from_str::<Value>(data_content) {
                    final_json = Some(parsed);
                }
                is_complete_event = false;
            }
        }
    }

    let json = final_json.ok_or("No complete event received from server")?;

    let b64 = json
        .get("image")
        .and_then(|v| v.as_str())
        .ok_or("Response JSON missing 'image' field")?;

    let image_bytes = general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("Base64 decode failed: {}", e))?;

    // Read metadata from the JSON (server always sends these)
    let width = json.get("width").and_then(|v| v.as_u64()).unwrap_or(512) as u32;
    let height = json.get("height").and_then(|v| v.as_u64()).unwrap_or(512) as u32;
    let channels = json.get("channels").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
    let format = json
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("raw")
        .to_string();

    println!(
        "Server format={} size={}x{} channels={} raw_bytes={}",
        format,
        width,
        height,
        channels,
        image_bytes.len()
    );

    Ok(GenerateResult {
        image_bytes,
        width,
        height,
        channels,
        format,
    })
}

/// Convert server response to a PNG byte vector
fn to_png(result: &GenerateResult) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let png_bytes = match result.format.as_str() {
        "raw" => {
            // Raw RGB (channels=3) or RGBA (channels=4) pixel bytes → encode as PNG
            let mut png_buf: Vec<u8> = Vec::new();
            if result.channels == 3 {
                let img: ImageBuffer<Rgb<u8>, _> =
                    ImageBuffer::from_raw(result.width, result.height, result.image_bytes.clone())
                        .ok_or("Failed to create RGB ImageBuffer — size mismatch")?;
                img.write_to(
                    &mut Cursor::new(&mut png_buf),
                    image::ImageOutputFormat::Png,
                )?;
            } else {
                return Err(format!("Unsupported channel count: {}", result.channels).into());
            }
            png_buf
        }
        // Already a compressed format — return as-is
        _ => result.image_bytes.clone(),
    };
    Ok(png_bytes)
}

/// Send raw RGB bytes to local-dream /upscale endpoint mirroring the mobile implementation
async fn upscale_image(
    raw_rgb_bytes: &[u8],
    width: u32,
    height: u32,
    absolute_upscaler_path: &str, // Expects full absolute path to upscaler.bin
    server_url: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    println!("Sending {} bytes to /upscale...", raw_rgb_bytes.len());

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/upscale", server_url))
        .header("X-Image-Width", width.to_string())
        .header("X-Image-Height", height.to_string())
        .header("X-Upscaler-Path", absolute_upscaler_path) // Full absolute path
        .header("Content-Type", "application/octet-stream")
        .body(raw_rgb_bytes.to_vec())
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response
            .text()
            .await
            .unwrap_or_else(|_| "(unreadable)".into());
        return Err(format!("Upscale returned {}: {}", status, body_text).into());
    }

    // According to the Kotlin protocol, the server outputs raw JPEG bytes
    let upscaled_bytes = response.bytes().await?;
    Ok(upscaled_bytes.to_vec())
}

/// Detect image format from magic bytes for saving with correct extension
fn detect_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else {
        "bin"
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all("outputs")?;

    let mut next_index = 0u32;
    if let Ok(entries) = fs::read_dir("outputs") {
        let mut indices: Vec<u32> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name();
                let name = name.to_str()?;
                if name.starts_with("image_") && !name.contains("upscaled") {
                    let stem = name.strip_prefix("image_")?;
                    let stem = stem.split('.').next()?;
                    stem.parse::<u32>().ok()
                } else {
                    None
                }
            })
            .collect();
        indices.sort_unstable();
        next_index = indices.into_iter().max().unwrap_or(0) + 1;
    }

    let server_url = "http://localhost:8081";
    let available_prompts = config::get_prompts();

    let params_list: Vec<HashMap<String, Value>> = available_prompts
        .into_iter()
        .map(|prompt_text| {
            let mut params = HashMap::new();
            params.insert("server_url".to_string(), json!(server_url));
            params.insert("prompt".to_string(), json!(prompt_text));
            params.insert(
                "negative_prompt".to_string(),
                json!(config::NEGATIVE_PROMPT),
            );
            params.insert("size".to_string(), json!(512));
            params.insert("steps".to_string(), json!(50));
            params.insert("cfg".to_string(), json!(7.0));
            params.insert("use_opencl".to_string(), json!(true));
            params.insert("scheduler".to_string(), json!("dpm_sde_karras"));
            params
        })
        .collect();

    let total_images = params_list.len();

    for (i, params) in params_list.iter().enumerate() {
        println!("\nGenerating image {}/{}", i + 1, total_images);

        match generate_image(params).await {
            Ok(result) => {
                let base_name = next_index + i as u32;

                // Convert raw RGB → PNG
                match to_png(&result) {
                    Ok(png_bytes) => {
                        // 1. Save original as PNG
                        let output_path = format!("outputs/image_{}.png", base_name);
                        let mut file = fs::File::create(&output_path)?;
                        file.write_all(&png_bytes)?;
                        println!("Saved original: {}", output_path);

                        // 2. Send original RAW RGB bytes to the upscaler
                        // Change this base string to match the absolute path of the model folder on your test environment
                        let upscaler_id = "upscaler_realistic";
                        let absolute_upscaler_path = format!(
                            "/data/user/0/io.github.xororz.localdream/files/models/{}/upscaler.bin",
                            upscaler_id
                        );

                        match upscale_image(
                            &result.image_bytes, // Use raw uncompressed RGB data
                            result.width,
                            result.height,
                            &absolute_upscaler_path,
                            server_url,
                        )
                        .await
                        {
                            Ok(upscaled_bytes) => {
                                let up_ext = detect_ext(&upscaled_bytes); // Should dynamically capture the incoming JPEG format
                                let upscale_path =
                                    format!("outputs/image_{}_upscaled.{}", base_name, up_ext);
                                let mut file = fs::File::create(&upscale_path)?;
                                file.write_all(&upscaled_bytes)?;
                                println!("Saved 4x upscaled: {}", upscale_path);
                            }
                            Err(e) => eprintln!("Upscale failed: {}", e),
                        }
                    }
                    Err(e) => eprintln!("PNG conversion failed: {}", e),
                }
            }
            Err(e) => eprintln!("Error generating image: {}", e),
        }
    }

    Ok(())
}
