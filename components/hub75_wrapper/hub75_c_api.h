#pragma once
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void* hub75_handle_t;

hub75_handle_t hub75_c_create(
    uint16_t width, uint16_t height,
    int r1, int g1, int b1,
    int r2, int g2, int b2,
    int a, int b, int c, int d, int e,
    int lat, int oe, int clk
);

bool hub75_c_begin(hub75_handle_t handle);
void hub75_c_draw_pixel(hub75_handle_t handle, uint16_t x, uint16_t y, uint8_t r, uint8_t g, uint8_t b);
void hub75_c_flip_buffer(hub75_handle_t handle);
void hub75_c_clear(hub75_handle_t handle);
void hub75_c_set_brightness(hub75_handle_t handle, uint8_t brightness);

#ifdef __cplusplus
}
#endif
