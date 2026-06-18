# Local Dream CLI Client

A fast, asynchronous Rust command-line tool designed to interface with the `local-dream` Android application backend. It reads configurations and text prompts from a local file, sends text-to-image requests to your phone over the local network via an event-stream API, downloads the generated raw image data, converts it to PNG, and automatically coordinates upscaling using the server's upscaler endpoint.

## Features

- **Runtime Configuration:** Prompts, negative prompts, dimensions, steps, schedulers, and upscaler configurations are fully loaded at runtime from a `config.toml` file—no recompilation needed to tweak settings or add prompts.
- **Asynchronous Event-Stream Streaming:** Uses fully asynchronous processing (`tokio` and `reqwest`) to monitor text/event-stream chunks safely until generation completes.
- **Auto Image Conversion:** Converts incoming raw format pixel byte streams into standard high-quality PNG buffers.
- **Automated Upscaling Pipeline:** Automatically chains generation into an upscaling workflow via multi-part form data uploads using target hardware upscaler configurations.
- **Smart Indexing:** Scans the `outputs/` directory automatically upon launch to increment save files properly (`image_1.png`, `image_1_upscaled.png`, etc.) without overwriting prior outputs.

## Prerequisites

- **Rust toolchain:** Ensure you have `cargo` and `rustc` installed (MSRV 1.56+ or edition 2021 compliant).
- **Active Backend:** The `local-dream` server must be actively running on your local device (default configuration expects `http://localhost:8081` via port forwarding or a local network endpoint). Model selection is controlled directly on the phone.

## Dependencies

This client utilizes standard Rust crate components for performance and network stability:
- `reqwest` & `futures-util` — Multi-part uploads, JSON body transport, and asynchronous event streams.
- `tokio` — Multi-threaded asynchronous execution runtime ecosystem.
- `serde`, `serde_json`, & `toml` — Deserialization of structural properties and runtime mappings.
- `image` & `base64` — Raster graphics pixel storage manipulation and string encoding handling.

## Installation & Setup
1. **Install Rust via Rustup (Linux/Mac OS):**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

2. **Clone the Repository:**
```bash
   git clone https://github.com/Lycra-GX/LD-Remote.git --depth=1
   cd LD-Remote
```

3. **Check Required Dependency:**
```bash
cargo --check
```

4. **Compile & Run:**
```bash
cargo run
```

## Troubleshooting

### Connection Errors (Device Can't Be Reached)
If you get a connection timeout error or the client cannot reach the server running on your phone, you need to map your local machine's port to your Android device via Android Debug Bridge (ADB).

1. **Check ADB Connection Status:**
   First, make sure your PC recognizes the device. If using a USB cable, run:
    ```bash
    adb devices
    ```
    The Output should be like this (I'm using wireless debugging).
    Ensure the output lists your device as device (and not unauthorized or offline).
    ```
    lycra@ourcastle:~/rust/local-dream$ adb devices
    List of devices attached
    192.168.0.142:33489     device
    ```

    If you are connecting over Wi-Fi (Wireless Debugging), establish the connection using the IP address and port shown on your phone:
    ```bash
    adb connect <device_ip_address>:<port>
    ```
2. **Forward the Server Port:**

    Once the connection is verified, execute the following command to route your local localhost:8081 traffic directly to the local-dream server on your phone:
    ```bash
    adb forward tcp:8081 tcp:8081
    ```

3. **Retry Runtime Execution:**

    After establishing the ADB port-forward tunnel, run 
    ```bash
    cargo run
    ```