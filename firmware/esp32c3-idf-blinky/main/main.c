/*
 * Ohmforge test firmware: an unmodified ESP-IDF blinky for the ESP32-C3.
 *
 * Built with ESP-IDF v5.3.2 and flashed as a normal factory app (partition
 * table at 0x8000, app at 0x10000). The simulator runs the full boot path:
 * ROM, the ESP-IDF second-stage bootloader, then this app from flash.
 * It prints a banner on UART0 and toggles GPIO8 every 500 µs.
 *
 * Copyright (C) 2026 Ohmforge contributors
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

#include <stdio.h>

#include "driver/gpio.h"
#include "esp_rom_sys.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

/** Half period: GPIO8 toggles this often (matches the esp-hal blinky). */
#define HALF_PERIOD_US 500
#define LED_GPIO GPIO_NUM_8

void app_main(void)
{
    gpio_config_t cfg = {
        .pin_bit_mask = 1ULL << LED_GPIO,
        .mode = GPIO_MODE_OUTPUT,
        .pull_up_en = GPIO_PULLUP_DISABLE,
        .pull_down_en = GPIO_PULLDOWN_DISABLE,
        .intr_type = GPIO_INTR_DISABLE,
    };
    ESP_ERROR_CHECK(gpio_config(&cfg));
    ESP_ERROR_CHECK(gpio_set_level(LED_GPIO, 0));

    printf("ohmforge idf blinky\n");
    fflush(stdout);

    int level = 0;
    for (;;) {
        level = !level;
        gpio_set_level(LED_GPIO, level);
        esp_rom_delay_us(HALF_PERIOD_US);
    }
}
