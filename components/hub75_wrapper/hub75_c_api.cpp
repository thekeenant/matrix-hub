#include "hub75_c_api.h"
#include "hub75.h"

extern "C" {

hub75_handle_t hub75_c_create(
    uint16_t width, uint16_t height,
    int r1, int g1, int b1,
    int r2, int g2, int b2,
    int a, int b, int c, int d, int e,
    int lat, int oe, int clk
) {
    Hub75Config config{};
    config.panel_width = width;
    config.panel_height = height;
    config.pins.r1 = r1;
    config.pins.g1 = g1;
    config.pins.b1 = b1;
    config.pins.r2 = r2;
    config.pins.g2 = g2;
    config.pins.b2 = b2;
    config.pins.a = a;
    config.pins.b = b;
    config.pins.c = c;
    config.pins.d = d;
    config.pins.e = e;
    config.pins.lat = lat;
    config.pins.oe = oe;
    config.pins.clk = clk;
    config.double_buffer = true; // Use double buffering
    config.brightness = 128;
    
    Hub75Driver* driver = new Hub75Driver(config);
    return static_cast<hub75_handle_t>(driver);
}

bool hub75_c_begin(hub75_handle_t handle) {
    if (!handle) return false;
    auto driver = static_cast<Hub75Driver*>(handle);
    return driver->begin();
}

void hub75_c_draw_pixel(hub75_handle_t handle, uint16_t x, uint16_t y, uint8_t r, uint8_t g, uint8_t b) {
    if (!handle) return;
    auto driver = static_cast<Hub75Driver*>(handle);
    driver->set_pixel(x, y, r, g, b);
}

void hub75_c_flip_buffer(hub75_handle_t handle) {
    if (!handle) return;
    auto driver = static_cast<Hub75Driver*>(handle);
    driver->flip_buffer();
}

void hub75_c_clear(hub75_handle_t handle) {
    if (!handle) return;
    auto driver = static_cast<Hub75Driver*>(handle);
    driver->clear();
}

void hub75_c_set_brightness(hub75_handle_t handle, uint8_t brightness) {
    if (!handle) return;
    auto driver = static_cast<Hub75Driver*>(handle);
    driver->set_brightness(brightness);
}

} // extern "C"
