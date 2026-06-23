# Matrix Hub

Matrix Hub is a Rust-based, dual-core embedded application designed for the ESP32 (using ESP-IDF) to drive a HUB75 LED matrix display. It acts as a smart display hub featuring various built-in apps, web-based configuration, and hardware gesture controls.

<video src="https://raw.githubusercontent.com/thekeenant/matrix-hub/main/assets/demo.mp4" controls="controls" muted="muted" width="100%"></video>


## Features

- **High-Performance Rendering**: Utilizes ESP32's dual cores. Core 1 is dedicated entirely to high-priority display driving and DMA memory management, ensuring smooth, flicker-free rendering. Core 0 handles application logic, networking, and asynchronous tasks.
- **Built-in Apps**:
  - **MTA**: Real-time subway arrivals and transit information.
  - **Plasma**: A mesmerizing, math-based plasma visualizer.
  - **Particles**: A physics-based particle system.
  - **Settings**: Device information and configuration.
- **Hardware Controls**:
  - **Gesture Support**: Integrated LIS3DH accelerometer for swipe and tilt gesture detection.
  - **Physical Buttons**: Up/Down GPIO buttons for manual navigation.
- **Network & Configuration**:
  - Built-in WiFi management with a Captive Portal for easy network setup.
  - A modern, responsive web frontend built with Preact and Vite for device configuration.
  - Protocol Buffers (protobuf) used for efficient communication.

## Hardware Requirements

- **Microcontroller**: ESP32 / ESP32-S3
- **Display**: HUB75 LED Matrix Display
- **Sensors**: LIS3DH Accelerometer (I2C)
- **Input**: Tactile buttons (GPIO)

### Default Pinout

| Component | Pin(s) |
| :--- | :--- |
| **I2C (LIS3DH)** | SDA: GPIO16, SCL: GPIO17 |
| **Buttons** | Up: GPIO6, Down: GPIO7 |

*(Refer to `src/config.rs` for the HUB75 display pin configuration).*

## Software Requirements

- [Rust](https://rustup.rs/) (v1.82+)
- `esp-idf` toolchain (installed via `esp-idf-sys` / `embuild`)
- [Node.js](https://nodejs.org/) & npm (for building the Preact frontend)

## Project Structure

- `src/`: The core Rust application.
  - `src/apps/`: Individual applications (MTA, Plasma, Particles, etc.).
  - `src/display/`: HUB75 matrix driver and framebuffer logic.
  - `src/input/`: Gesture and button handling.
  - `src/network/`: WiFi, DNS (Captive Portal), and HTTP server logic.
- `frontend/`: Preact/Vite based web application for device configuration.
- `proto/`: Protocol Buffer definitions for data exchange.
- `components/`: ESP-IDF native components (e.g., HUB75 wrapper).

## Building and Running

1. **Build the Frontend**:
   Before compiling the Rust code, you need to build the Preact frontend so its assets can be embedded or served by the device.
   ```bash
   cd frontend
   npm install
   npm run build
   cd ..
   ```

2. **Build and Flash the ESP32**:
   Ensure you have the Rust ESP toolchain set up.
   ```bash
   cargo build --release
   cargo run --release
   ```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
